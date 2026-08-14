use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, warn};
use url::Url;

use crate::{
    AggregatedHit, EngineCredentials, EngineKind, EngineOutput, Error, MetaSearchOutput, Result,
    SearchEngine, SearchHit, SearchQuery,
    engines::{
        Bing, Brave, BraveWeb, CratesIo, Crossref, DockerHub, DuckDuckGo, GitHub, Google,
        GoogleCse, HackerNews, HuggingFace, Mwmbl, Npm, Nvd, OpenAlex, OpenLibrary, PubMed,
        Wikidata, Wikipedia, Yahoo,
    },
};

const RRF_K: f64 = 60.0;

#[derive(Debug, Clone)]
pub struct MetaSearchConfig {
    pub collection_window: Duration,
    pub overall_timeout: Duration,
    pub cache_ttl: Duration,
    pub max_cache_entries: usize,
    pub per_engine_concurrency: usize,
    pub failure_threshold: usize,
    pub engine_cooldown: Duration,
    /// Applied instead of `engine_cooldown` when an engine reports a rate
    /// limit, and applied on the first such failure rather than after
    /// `failure_threshold`.
    ///
    /// A timeout is worth retrying in half a minute; a block is not. Engines
    /// that gate on bot detection block the caller's address, not a session,
    /// and retrying inside the block extends it, so a short cooldown turns one
    /// refusal into a standing one.
    pub rate_limit_cooldown: Duration,
}

impl Default for MetaSearchConfig {
    fn default() -> Self {
        Self {
            collection_window: Duration::from_millis(900),
            overall_timeout: Duration::from_secs(6),
            cache_ttl: Duration::from_secs(60),
            max_cache_entries: 256,
            per_engine_concurrency: 4,
            failure_threshold: 2,
            engine_cooldown: Duration::from_secs(30),
            rate_limit_cooldown: Duration::from_secs(3600),
        }
    }
}

pub struct MetaSearch {
    config: MetaSearchConfig,
    engines: Vec<Arc<EngineSlot>>,
    cache: RwLock<HashMap<CacheKey, CacheEntry>>,
}

impl MetaSearch {
    pub fn new() -> Result<Self> {
        Self::from_engine_kinds(MetaSearchConfig::default(), &EngineKind::ALL)
    }

    pub fn from_engine_kinds(config: MetaSearchConfig, kinds: &[EngineKind]) -> Result<Self> {
        Self::from_engine_kinds_with_credentials(config, kinds, &EngineCredentials::default())
    }

    pub fn from_engine_kinds_with_credentials(
        config: MetaSearchConfig,
        kinds: &[EngineKind],
        credentials: &EngineCredentials,
    ) -> Result<Self> {
        let mut enabled = Vec::new();
        let mut engines: Vec<(EngineKind, Arc<dyn SearchEngine>)> = Vec::new();
        for kind in kinds {
            if enabled.contains(kind) {
                continue;
            }
            enabled.push(*kind);
            let engine: Arc<dyn SearchEngine> = match kind {
                EngineKind::Bing => Arc::new(Bing::new()?),
                EngineKind::Brave => Arc::new(Brave::new(required_credential(
                    &credentials.brave_api_key,
                    "Brave API key",
                )?)?),
                EngineKind::BraveWeb => Arc::new(BraveWeb::new()?),
                EngineKind::CratesIo => Arc::new(CratesIo::new()?),
                EngineKind::Crossref => Arc::new(Crossref::new()?),
                EngineKind::DockerHub => Arc::new(DockerHub::new()?),
                EngineKind::DuckDuckGo => Arc::new(DuckDuckGo::new()?),
                EngineKind::GitHub => Arc::new(GitHub::new()?),
                EngineKind::Google => Arc::new(Google::new(
                    required_credential(&credentials.google_api_key, "Google API key")?,
                    required_credential(
                        &credentials.google_search_engine_id,
                        "Google Programmable Search Engine ID",
                    )?,
                )?),
                EngineKind::GoogleCse => Arc::new(GoogleCse::new()?),
                EngineKind::HackerNews => Arc::new(HackerNews::new()?),
                EngineKind::HuggingFace => Arc::new(HuggingFace::new()?),
                EngineKind::Mwmbl => Arc::new(Mwmbl::new()?),
                EngineKind::Npm => Arc::new(Npm::new()?),
                EngineKind::Nvd => Arc::new(Nvd::new()?),
                EngineKind::OpenAlex => Arc::new(OpenAlex::new()?),
                EngineKind::OpenLibrary => Arc::new(OpenLibrary::new()?),
                EngineKind::PubMed => Arc::new(PubMed::new()?),
                EngineKind::Wikidata => Arc::new(Wikidata::new()?),
                EngineKind::Wikipedia => Arc::new(Wikipedia::new()?),
                EngineKind::Yahoo => Arc::new(Yahoo::new()?),
            };
            engines.push((*kind, engine));
        }
        if engines.is_empty() {
            return Err(Error::InvalidConfiguration(
                "at least one engine must be enabled".into(),
            ));
        }
        Ok(Self::with_engine_kinds(config, engines))
    }

    pub fn with_engines(config: MetaSearchConfig, engines: Vec<Arc<dyn SearchEngine>>) -> Self {
        let concurrency = config.per_engine_concurrency.max(1);
        Self {
            engines: engines
                .into_iter()
                .map(|engine| Arc::new(EngineSlot::new(None, engine, concurrency)))
                .collect(),
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    fn with_engine_kinds(
        config: MetaSearchConfig,
        engines: Vec<(EngineKind, Arc<dyn SearchEngine>)>,
    ) -> Self {
        let concurrency = config.per_engine_concurrency.max(1);
        Self {
            engines: engines
                .into_iter()
                .map(|(kind, engine)| Arc::new(EngineSlot::new(Some(kind), engine, concurrency)))
                .collect(),
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<MetaSearchOutput> {
        self.search_selected(query, None).await
    }

    pub async fn search_with_engine_kinds(
        &self,
        query: &SearchQuery,
        engines: &[EngineKind],
    ) -> Result<MetaSearchOutput> {
        if engines.is_empty() {
            return Err(Error::InvalidQuery(
                "at least one search engine must be selected".into(),
            ));
        }
        self.search_selected(query, Some(engines)).await
    }

    async fn search_selected(
        &self,
        query: &SearchQuery,
        selected: Option<&[EngineKind]>,
    ) -> Result<MetaSearchOutput> {
        validate(query)?;
        let started = Instant::now();
        let mut selected_names: Vec<&str> = selected
            .map(|engines| engines.iter().map(|engine| engine.as_str()).collect())
            .unwrap_or_else(|| self.engines.iter().map(|slot| slot.engine.name()).collect());
        selected_names.sort_unstable();
        selected_names.dedup();
        let cache_key = CacheKey::new(query, &selected_names);
        if let Some(mut output) = self.cached(&cache_key).await {
            output.cached = true;
            output.elapsed = started.elapsed();
            return Ok(output);
        }

        let available = self
            .engines
            .iter()
            .filter(|slot| {
                selected.is_none_or(|engines| slot.kind.is_some_and(|kind| engines.contains(&kind)))
            })
            .filter(|slot| slot.available())
            .cloned()
            .collect::<Vec<_>>();
        if available.is_empty() {
            return Err(Error::AllEnginesFailed(
                "selected engines are unavailable or cooling down".into(),
            ));
        }

        let attempted = available
            .iter()
            .map(|slot| slot.engine.name())
            .collect::<Vec<_>>();
        let mut tasks = FuturesUnordered::new();
        for slot in available {
            let query = query.clone();
            tasks.push(async move {
                let result = {
                    let permit = slot.limit.acquire().await;
                    match permit {
                        Ok(_permit) => slot.engine.search(&query).await,
                        Err(_) => Err(Error::Request {
                            engine: slot.engine.name(),
                            message: "engine concurrency limiter closed".into(),
                        }),
                    }
                };
                (slot, result)
            });
        }

        let hard_deadline = started + self.config.overall_timeout;
        let mut collection_deadline = None;
        let mut outputs = Vec::new();
        let mut failures = Vec::new();
        while !tasks.is_empty() {
            let deadline = collection_deadline
                .unwrap_or(hard_deadline)
                .min(hard_deadline);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(Some((slot, result))) = tokio::time::timeout(remaining, tasks.next()).await
            else {
                break;
            };
            match result {
                Ok(output) => {
                    slot.record_success();
                    if !output.hits.is_empty() && collection_deadline.is_none() {
                        collection_deadline = Some(Instant::now() + self.config.collection_window);
                    }
                    outputs.push((slot.engine.weight(), output));
                }
                Err(error) => {
                    warn!(engine = slot.engine.name(), %error, "metasearch engine failed");
                    slot.record_failure(&self.config, &error);
                    failures.push(error.to_string());
                }
            }
        }

        if outputs.is_empty() {
            return Err(Error::AllEnginesFailed(if failures.is_empty() {
                "overall search deadline elapsed".into()
            } else {
                failures.join("; ")
            }));
        }

        let completed = outputs
            .iter()
            .map(|(_, output)| output.engine)
            .collect::<Vec<_>>();
        for engine in attempted {
            if !completed.contains(&engine)
                && !failures.iter().any(|failure| failure.starts_with(engine))
            {
                debug!(
                    engine,
                    "metasearch collection window elapsed before engine completed"
                );
            }
        }
        let hits = aggregate(outputs, query.limit);
        let output = MetaSearchOutput {
            engines_used: completed.into_iter().map(str::to_owned).collect(),
            engine_failures: failures,
            hits,
            elapsed: started.elapsed(),
            cached: false,
        };
        self.store(cache_key, output.clone()).await;
        Ok(output)
    }

    async fn cached(&self, key: &CacheKey) -> Option<MetaSearchOutput> {
        let cache = self.cache.read().await;
        cache
            .get(key)
            .filter(|entry| entry.stored_at.elapsed() < self.config.cache_ttl)
            .map(|entry| entry.output.clone())
    }

    async fn store(&self, key: CacheKey, output: MetaSearchOutput) {
        let mut cache = self.cache.write().await;
        cache.retain(|_, entry| entry.stored_at.elapsed() < self.config.cache_ttl);
        if cache.len() >= self.config.max_cache_entries.max(1)
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.stored_at)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            CacheEntry {
                stored_at: Instant::now(),
                output,
            },
        );
    }
}

impl Default for MetaSearch {
    fn default() -> Self {
        Self::new().expect("default metasearch engines are valid")
    }
}

struct EngineSlot {
    kind: Option<EngineKind>,
    engine: Arc<dyn SearchEngine>,
    limit: Semaphore,
    state: Mutex<EngineState>,
}

impl EngineSlot {
    fn new(kind: Option<EngineKind>, engine: Arc<dyn SearchEngine>, concurrency: usize) -> Self {
        Self {
            kind,
            engine,
            limit: Semaphore::new(concurrency),
            state: Mutex::new(EngineState::default()),
        }
    }

    fn available(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("engine state lock is not poisoned");
        if state
            .disabled_until
            .is_some_and(|until| until > Instant::now())
        {
            return false;
        }
        *state = EngineState::default();
        true
    }

    fn record_success(&self) {
        let mut state = self
            .state
            .lock()
            .expect("engine state lock is not poisoned");
        *state = EngineState::default();
    }

    fn record_failure(&self, config: &MetaSearchConfig, error: &Error) {
        let mut state = self
            .state
            .lock()
            .expect("engine state lock is not poisoned");
        state.consecutive_failures += 1;
        if matches!(error, Error::RateLimited { .. }) {
            state.disabled_until = Some(Instant::now() + config.rate_limit_cooldown);
        } else if state.consecutive_failures >= config.failure_threshold.max(1) {
            state.disabled_until = Some(Instant::now() + config.engine_cooldown);
        }
    }
}

#[derive(Default)]
struct EngineState {
    consecutive_failures: usize,
    disabled_until: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    query: String,
    limit: usize,
    country: String,
    language: String,
    engines: Vec<String>,
}

impl CacheKey {
    fn new(query: &SearchQuery, engines: &[&str]) -> Self {
        Self {
            query: query
                .query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            limit: query.limit,
            country: query.country.to_lowercase(),
            language: query.language.to_lowercase(),
            engines: engines.iter().map(|engine| (*engine).into()).collect(),
        }
    }
}

struct CacheEntry {
    stored_at: Instant,
    output: MetaSearchOutput,
}

struct RankedHit {
    hit: SearchHit,
    sources: Vec<String>,
    score: f64,
}

fn aggregate(outputs: Vec<(f64, EngineOutput)>, limit: usize) -> Vec<AggregatedHit> {
    let mut ranked = HashMap::<String, RankedHit>::new();
    for (weight, output) in outputs {
        for (index, hit) in output.hits.into_iter().enumerate() {
            let Some(key) = normalized_url(&hit.url) else {
                continue;
            };
            let score = weight / (RRF_K + index as f64 + 1.0);
            match ranked.get_mut(&key) {
                Some(existing) => {
                    existing.score += score;
                    if !existing
                        .sources
                        .iter()
                        .any(|source| source == output.engine)
                    {
                        existing.sources.push(output.engine.into());
                    }
                    if hit.snippet.as_ref().map_or(0, String::len)
                        > existing.hit.snippet.as_ref().map_or(0, String::len)
                    {
                        existing.hit.snippet = hit.snippet;
                    }
                }
                None => {
                    ranked.insert(
                        key,
                        RankedHit {
                            hit,
                            sources: vec![output.engine.into()],
                            score,
                        },
                    );
                }
            }
        }
    }
    let mut ranked = ranked.into_values().collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.hit.title.cmp(&right.hit.title))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|ranked| AggregatedHit {
            title: ranked.hit.title,
            url: ranked.hit.url,
            snippet: ranked.hit.snippet,
            sources: ranked.sources,
            score: ranked.score,
        })
        .collect()
}

fn normalized_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    let mut pairs = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "fbclid" | "gclid" | "msclkid")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    Some(url.to_string())
}

fn required_credential(value: &Option<String>, name: &str) -> Result<String> {
    value
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| Error::InvalidConfiguration(format!("{name} is required")))
}

fn validate(query: &SearchQuery) -> Result<()> {
    if query.query.trim().is_empty() {
        return Err(Error::InvalidQuery("query cannot be empty".into()));
    }
    if query.query.len() > 512 {
        return Err(Error::InvalidQuery("query cannot exceed 512 bytes".into()));
    }
    if !(1..=20).contains(&query.limit) {
        return Err(Error::InvalidQuery("limit must be between 1 and 20".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxSearchFuture;

    #[test]
    fn engine_policy_is_applied_and_deduplicated() {
        let search = MetaSearch::from_engine_kinds(
            MetaSearchConfig::default(),
            &[EngineKind::Wikipedia, EngineKind::Wikipedia],
        )
        .unwrap();
        assert_eq!(search.engines.len(), 1);
        assert_eq!(search.engines[0].engine.name(), "wikipedia");
    }

    #[test]
    fn credential_free_brave_web_requires_no_configuration() {
        let search =
            MetaSearch::from_engine_kinds(MetaSearchConfig::default(), &[EngineKind::BraveWeb])
                .unwrap();
        assert_eq!(search.engines[0].engine.name(), "brave-web");
    }

    #[test]
    fn engine_policy_cannot_be_empty() {
        assert!(MetaSearch::from_engine_kinds(MetaSearchConfig::default(), &[]).is_err());
    }

    #[test]
    fn credentialed_engines_require_configuration() {
        assert!(
            MetaSearch::from_engine_kinds(MetaSearchConfig::default(), &[EngineKind::Brave])
                .is_err()
        );
        let credentials = EngineCredentials {
            brave_api_key: Some("brave-key".into()),
            google_api_key: Some("google-key".into()),
            google_search_engine_id: Some("search-engine-id".into()),
        };
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("brave-key"));
        assert!(!debug.contains("google-key"));
        assert!(!debug.contains("search-engine-id"));
        let search = MetaSearch::from_engine_kinds_with_credentials(
            MetaSearchConfig::default(),
            &[EngineKind::Brave, EngineKind::Google],
            &credentials,
        )
        .unwrap();
        let names = search
            .engines
            .iter()
            .map(|engine| engine.engine.name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["brave", "google"]);
    }

    struct FakeEngine {
        name: &'static str,
        delay: Duration,
        hits: Vec<SearchHit>,
    }

    impl SearchEngine for FakeEngine {
        fn name(&self) -> &'static str {
            self.name
        }

        fn search<'a>(&'a self, _query: &'a SearchQuery) -> BoxSearchFuture<'a> {
            Box::pin(async move {
                tokio::time::sleep(self.delay).await;
                Ok(EngineOutput {
                    engine: self.name,
                    hits: self.hits.clone(),
                    elapsed: self.delay,
                })
            })
        }
    }

    fn hit(url: &str, title: &str) -> SearchHit {
        SearchHit {
            title: title.into(),
            url: url.into(),
            snippet: None,
        }
    }

    #[tokio::test]
    async fn queries_engines_concurrently_and_merges_duplicates() {
        let engines: Vec<Arc<dyn SearchEngine>> = vec![
            Arc::new(FakeEngine {
                name: "one",
                delay: Duration::from_millis(10),
                hits: vec![hit("https://example.com/?utm_source=test", "Example")],
            }),
            Arc::new(FakeEngine {
                name: "two",
                delay: Duration::from_millis(20),
                hits: vec![hit("https://example.com/", "Example Domain")],
            }),
            Arc::new(FakeEngine {
                name: "slow",
                delay: Duration::from_secs(1),
                hits: vec![hit("https://slow.example/", "Slow")],
            }),
        ];
        let search = MetaSearch::with_engines(
            MetaSearchConfig {
                collection_window: Duration::from_millis(30),
                overall_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            engines,
        );
        let started = Instant::now();
        let output = search
            .search(&SearchQuery::new("example", 5))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(output.hits.len(), 1);
        assert_eq!(output.hits[0].sources, vec!["one", "two"]);
        assert_eq!(output.engines_used, vec!["one", "two"]);
    }

    #[tokio::test]
    async fn selects_requested_engines_and_separates_cache_entries() {
        let engines: Vec<(EngineKind, Arc<dyn SearchEngine>)> = vec![
            (
                EngineKind::Bing,
                Arc::new(FakeEngine {
                    name: "bing",
                    delay: Duration::from_millis(1),
                    hits: vec![hit("https://bing.example/", "Bing")],
                }),
            ),
            (
                EngineKind::Wikipedia,
                Arc::new(FakeEngine {
                    name: "wikipedia",
                    delay: Duration::from_millis(1),
                    hits: vec![hit("https://wikipedia.example/", "Wikipedia")],
                }),
            ),
        ];
        let search = MetaSearch::with_engine_kinds(MetaSearchConfig::default(), engines);
        let query = SearchQuery::new("example", 5);

        let bing = search
            .search_with_engine_kinds(&query, &[EngineKind::Bing])
            .await
            .unwrap();
        assert_eq!(bing.engines_used, ["bing"]);
        assert_eq!(bing.hits[0].title, "Bing");

        let wikipedia = search
            .search_with_engine_kinds(&query, &[EngineKind::Wikipedia])
            .await
            .unwrap();
        assert!(!wikipedia.cached);
        assert_eq!(wikipedia.engines_used, ["wikipedia"]);
        assert_eq!(wikipedia.hits[0].title, "Wikipedia");

        let combined = search
            .search_with_engine_kinds(&query, &[EngineKind::Bing, EngineKind::Wikipedia])
            .await
            .unwrap();
        assert!(!combined.cached);
        let reordered = search
            .search_with_engine_kinds(
                &query,
                &[EngineKind::Wikipedia, EngineKind::Bing, EngineKind::Bing],
            )
            .await
            .unwrap();
        assert!(reordered.cached);
    }

    #[tokio::test]
    async fn empty_results_are_a_successful_search() {
        let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(FakeEngine {
            name: "empty",
            delay: Duration::from_millis(1),
            hits: Vec::new(),
        })];
        let search = MetaSearch::with_engines(MetaSearchConfig::default(), engines);
        let output = search
            .search(&SearchQuery::new("no matches", 5))
            .await
            .unwrap();
        assert!(output.hits.is_empty());
        assert!(output.engine_failures.is_empty());
        assert_eq!(output.engines_used, ["empty"]);
    }

    #[test]
    fn expired_cooldown_resets_failure_count() {
        let slot = EngineSlot::new(
            None,
            Arc::new(FakeEngine {
                name: "test",
                delay: Duration::ZERO,
                hits: Vec::new(),
            }),
            1,
        );
        let config = MetaSearchConfig {
            failure_threshold: 2,
            engine_cooldown: Duration::ZERO,
            ..Default::default()
        };
        let error = Error::Timeout { engine: "test" };
        slot.record_failure(&config, &error);
        slot.record_failure(&config, &error);
        assert!(slot.available());
        slot.record_failure(&config, &error);
        assert!(slot.available());
    }

    #[test]
    fn a_rate_limit_disables_the_engine_immediately() {
        let slot = EngineSlot::new(
            None,
            Arc::new(FakeEngine {
                name: "test",
                delay: Duration::ZERO,
                hits: Vec::new(),
            }),
            1,
        );
        let config = MetaSearchConfig {
            failure_threshold: 2,
            engine_cooldown: Duration::ZERO,
            rate_limit_cooldown: Duration::from_secs(3600),
            ..Default::default()
        };
        // One refusal is enough. Waiting for the threshold would mean sending a
        // second request into a block that the first one already reported, and
        // the engines that block do so by address rather than by session.
        slot.record_failure(&config, &Error::RateLimited { engine: "test" });
        assert!(!slot.available());
    }

    #[tokio::test]
    async fn caches_identical_queries() {
        let engines: Vec<Arc<dyn SearchEngine>> = vec![Arc::new(FakeEngine {
            name: "one",
            delay: Duration::from_millis(1),
            hits: vec![hit("https://example.com/", "Example")],
        })];
        let search = MetaSearch::with_engines(MetaSearchConfig::default(), engines);
        let query = SearchQuery::new("example", 1);
        assert!(!search.search(&query).await.unwrap().cached);
        assert!(search.search(&query).await.unwrap().cached);
    }

    #[tokio::test]
    #[ignore = "queries public search services"]
    async fn live_metasearch_returns_without_waiting_for_every_engine() {
        let search = MetaSearch::new().unwrap();
        let output = search
            .search(&SearchQuery::new("Rust programming language", 5))
            .await
            .unwrap();
        assert!(!output.hits.is_empty());
        assert!(output.elapsed < Duration::from_secs(3));
        assert!(output.engines_used.iter().any(|engine| engine == "bing"));
    }
}
