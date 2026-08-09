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
    engines::{Bing, Brave, DuckDuckGo, Google, Naver, Wikipedia},
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
        let mut engines: Vec<Arc<dyn SearchEngine>> = Vec::new();
        for kind in kinds {
            if enabled.contains(kind) {
                continue;
            }
            enabled.push(*kind);
            match kind {
                EngineKind::Bing => engines.push(Arc::new(Bing::new()?)),
                EngineKind::Brave => engines.push(Arc::new(Brave::new(required_credential(
                    &credentials.brave_api_key,
                    "Brave API key",
                )?)?)),
                EngineKind::DuckDuckGo => engines.push(Arc::new(DuckDuckGo::new()?)),
                EngineKind::Google => engines.push(Arc::new(Google::new(
                    required_credential(&credentials.google_api_key, "Google API key")?,
                    required_credential(
                        &credentials.google_search_engine_id,
                        "Google Programmable Search Engine ID",
                    )?,
                )?)),
                EngineKind::Naver => engines.push(Arc::new(Naver::new()?)),
                EngineKind::Wikipedia => engines.push(Arc::new(Wikipedia::new()?)),
            }
        }
        if engines.is_empty() {
            return Err(Error::InvalidConfiguration(
                "at least one engine must be enabled".into(),
            ));
        }
        Ok(Self::with_engines(config, engines))
    }

    pub fn with_engines(config: MetaSearchConfig, engines: Vec<Arc<dyn SearchEngine>>) -> Self {
        let concurrency = config.per_engine_concurrency.max(1);
        Self {
            engines: engines
                .into_iter()
                .map(|engine| Arc::new(EngineSlot::new(engine, concurrency)))
                .collect(),
            config,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<MetaSearchOutput> {
        validate(query)?;
        let started = Instant::now();
        let cache_key = CacheKey::new(query);
        if let Some(mut output) = self.cached(&cache_key).await {
            output.cached = true;
            output.elapsed = started.elapsed();
            return Ok(output);
        }

        let available = self
            .engines
            .iter()
            .filter(|slot| slot.available())
            .cloned()
            .collect::<Vec<_>>();
        if available.is_empty() {
            return Err(Error::AllEnginesFailed(
                "all engines are cooling down".into(),
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
                Ok(output) if !output.hits.is_empty() => {
                    slot.record_success();
                    if collection_deadline.is_none() {
                        collection_deadline = Some(Instant::now() + self.config.collection_window);
                    }
                    outputs.push((slot.engine.weight(), output));
                }
                Ok(_) => {
                    let message = format!("{} returned no results", slot.engine.name());
                    slot.record_failure(&self.config);
                    failures.push(message);
                }
                Err(error) => {
                    warn!(engine = slot.engine.name(), %error, "metasearch engine failed");
                    slot.record_failure(&self.config);
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
    engine: Arc<dyn SearchEngine>,
    limit: Semaphore,
    state: Mutex<EngineState>,
}

impl EngineSlot {
    fn new(engine: Arc<dyn SearchEngine>, concurrency: usize) -> Self {
        Self {
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
        state.disabled_until = None;
        true
    }

    fn record_success(&self) {
        let mut state = self
            .state
            .lock()
            .expect("engine state lock is not poisoned");
        *state = EngineState::default();
    }

    fn record_failure(&self, config: &MetaSearchConfig) {
        let mut state = self
            .state
            .lock()
            .expect("engine state lock is not poisoned");
        state.consecutive_failures += 1;
        if state.consecutive_failures >= config.failure_threshold.max(1) {
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
}

impl CacheKey {
    fn new(query: &SearchQuery) -> Self {
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
