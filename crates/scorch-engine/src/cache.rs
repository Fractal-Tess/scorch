use std::{
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    mem::size_of,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use scorch_types::{ScrapeDocument, ScrapeFormat, ScrapeOptions};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::{
    browser::RenderedPage,
    error::{EngineError, Result},
    extract,
    extract::ExtractInput,
};

pub(crate) const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHE_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const CACHE_WARM_QUEUE_CAPACITY: usize = 8;
const MAX_CACHE_WARM_BYTES: usize = 16 * 1024 * 1024;
const MAX_OBSERVATION_TOMBSTONES: usize = 1_024;
const MAX_RENDER_FLIGHTS: usize = 1_024;
const ALL_FORMATS: [ScrapeFormat; 5] = [
    ScrapeFormat::Markdown,
    ScrapeFormat::Html,
    ScrapeFormat::Text,
    ScrapeFormat::Links,
    ScrapeFormat::Metadata,
];
const ALL_FORMATS_MASK: u8 = (1 << ALL_FORMATS.len()) - 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ScrapeCacheKey {
    url: String,
    wait_for_ms: u64,
    only_main_content: bool,
    block_media: bool,
}

impl ScrapeCacheKey {
    pub(super) fn new(url: &str, options: &ScrapeOptions) -> Self {
        let url = url::Url::parse(url)
            .map(|parsed| parsed.to_string())
            .unwrap_or_else(|_| url.to_owned());
        Self {
            url,
            wait_for_ms: options.wait_for_ms,
            only_main_content: options.only_main_content,
            block_media: options.block_media,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct RenderFlightKey {
    url: String,
    wait_for_ms: u64,
    block_media: bool,
    timeout_ms: u64,
}

impl RenderFlightKey {
    pub(super) fn new(url: &str, options: &ScrapeOptions) -> Self {
        Self {
            url: url::Url::parse(url)
                .map(|parsed| parsed.to_string())
                .unwrap_or_else(|_| url.to_owned()),
            wait_for_ms: options.wait_for_ms,
            block_media: options.block_media,
            timeout_ms: options.timeout_ms,
        }
    }
}

#[derive(Clone)]
pub(super) enum RenderFlightOutcome {
    Rendered(RenderedPage),
    Failed(EngineError),
    RetryCache,
}

#[derive(Default)]
struct RenderFlightCacheState {
    generation: Option<u64>,
    invalidated: bool,
    published: bool,
}

pub(super) struct RenderFlightEntry {
    sender: watch::Sender<Option<Arc<RenderFlightOutcome>>>,
    leases: AtomicUsize,
    completed: AtomicBool,
    cancellation: CancellationToken,
    cache: Mutex<HashMap<ScrapeCacheKey, RenderFlightCacheState>>,
}

#[derive(Default)]
pub(super) struct RenderFlights {
    entries: Mutex<HashMap<RenderFlightKey, Arc<RenderFlightEntry>>>,
}

pub(super) struct RenderFlightAdmission {
    pub lease: RenderFlightLease,
    pub is_leader: bool,
}

pub(super) struct RenderFlightLease {
    flights: Arc<RenderFlights>,
    key: RenderFlightKey,
    entry: Arc<RenderFlightEntry>,
    receiver: watch::Receiver<Option<Arc<RenderFlightOutcome>>>,
}

impl RenderFlights {
    pub(super) fn acquire(self: &Arc<Self>, key: RenderFlightKey) -> Option<RenderFlightAdmission> {
        let mut entries = self.entries.lock().ok()?;
        if let Some(entry) = entries.get(&key) {
            entry.leases.fetch_add(1, Ordering::Relaxed);
            return Some(RenderFlightAdmission {
                lease: RenderFlightLease {
                    flights: Arc::clone(self),
                    key,
                    entry: Arc::clone(entry),
                    receiver: entry.sender.subscribe(),
                },
                is_leader: false,
            });
        }
        if entries.len() >= MAX_RENDER_FLIGHTS {
            return None;
        }
        let (sender, receiver) = watch::channel(None);
        let entry = Arc::new(RenderFlightEntry {
            sender,
            leases: AtomicUsize::new(1),
            completed: AtomicBool::new(false),
            cancellation: CancellationToken::new(),
            cache: Mutex::new(HashMap::new()),
        });
        entries.insert(key.clone(), Arc::clone(&entry));
        Some(RenderFlightAdmission {
            lease: RenderFlightLease {
                flights: Arc::clone(self),
                key,
                entry,
                receiver,
            },
            is_leader: true,
        })
    }

    pub(super) fn complete(
        &self,
        key: &RenderFlightKey,
        entry: &Arc<RenderFlightEntry>,
        outcome: RenderFlightOutcome,
    ) {
        entry.sender.send_replace(Some(Arc::new(outcome)));
        entry.completed.store(true, Ordering::Release);
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entry.leases.load(Ordering::Acquire) == 0
            && entries
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            entries.remove(key);
        }
    }

    pub(super) fn retry_cache(&self, key: &RenderFlightKey, entry: &Arc<RenderFlightEntry>) {
        if let Ok(mut entries) = self.entries.lock()
            && entries
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            entries.remove(key);
        }
        entry.completed.store(true, Ordering::Release);
        entry
            .sender
            .send_replace(Some(Arc::new(RenderFlightOutcome::RetryCache)));
    }

    fn release(&self, key: &RenderFlightKey, entry: &Arc<RenderFlightEntry>) {
        let Ok(mut entries) = self.entries.lock() else {
            entry.leases.fetch_sub(1, Ordering::AcqRel);
            return;
        };
        let remaining = entry.leases.fetch_sub(1, Ordering::AcqRel) - 1;
        if remaining != 0
            || !entries
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            return;
        }
        entries.remove(key);
        if !entry.completed.load(Ordering::Acquire) {
            entry.cancellation.cancel();
        }
    }
}

impl RenderFlightLease {
    pub(super) fn producer(&self) -> (RenderFlightKey, Arc<RenderFlightEntry>, CancellationToken) {
        (
            self.key.clone(),
            Arc::clone(&self.entry),
            self.entry.cancellation.clone(),
        )
    }

    pub(super) async fn wait(&mut self) -> Result<Arc<RenderFlightOutcome>> {
        loop {
            if let Some(outcome) = self.receiver.borrow().as_ref() {
                return Ok(Arc::clone(outcome));
            }
            if self.receiver.changed().await.is_err() {
                return Err(EngineError::Browser(
                    "coalesced browser render stopped unexpectedly".into(),
                ));
            }
        }
    }

    pub(super) fn prepare_cache_observation(
        &self,
        cache: &ScrapeCache,
        cache_key: &ScrapeCacheKey,
    ) -> Option<u64> {
        let mut states = self.entry.cache.lock().ok()?;
        let state = states.entry(cache_key.clone()).or_default();
        let generation = *state
            .generation
            .get_or_insert_with(|| cache.begin_observation(cache_key));
        if !state.invalidated {
            cache.invalidate_observation(cache_key, generation);
            state.invalidated = true;
        }
        Some(generation)
    }

    pub(super) fn claim_cache_publication(&self, cache_key: &ScrapeCacheKey) -> bool {
        let Ok(mut states) = self.entry.cache.lock() else {
            return false;
        };
        let state = states.entry(cache_key.clone()).or_default();
        if state.published {
            return false;
        }
        state.published = true;
        true
    }
}

impl Drop for RenderFlightLease {
    fn drop(&mut self) {
        self.flights.release(&self.key, &self.entry);
    }
}

struct CacheEntry {
    document: Arc<ScrapeDocument>,
    formats: u8,
    observed_at: Instant,
    expires_at: Instant,
    size: usize,
    generation: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<ScrapeCacheKey, CacheEntry>,
    order: VecDeque<ScrapeCacheKey>,
    latest_observations: HashMap<u64, u64>,
    observation_order: VecDeque<u64>,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct ScrapeCache {
    state: Mutex<CacheState>,
    next_generation: AtomicU64,
}

impl ScrapeCache {
    pub(super) fn can_store(
        &self,
        key: &ScrapeCacheKey,
        document: &ScrapeDocument,
        ttl: Duration,
    ) -> bool {
        !ttl.is_zero() && entry_size(key, document) <= MAX_CACHE_ENTRY_BYTES
    }

    pub(super) fn begin_observation(&self, key: &ScrapeCacheKey) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let key_hash = cache_key_hash(key);
        if let Ok(mut state) = self.state.lock() {
            state.latest_observations.insert(key_hash, generation);
            state
                .observation_order
                .retain(|candidate| *candidate != key_hash);
            state.observation_order.push_back(key_hash);
            while state.latest_observations.len() > MAX_OBSERVATION_TOMBSTONES {
                let Some(oldest) = state.observation_order.pop_front() else {
                    break;
                };
                state.latest_observations.remove(&oldest);
                let stale_keys = state
                    .entries
                    .keys()
                    .filter(|key| cache_key_hash(key) == oldest)
                    .cloned()
                    .collect::<Vec<_>>();
                for stale_key in stale_keys {
                    remove_entry(&mut state, &stale_key);
                }
            }
        }
        generation
    }

    pub(super) fn invalidate_observation(&self, key: &ScrapeCacheKey, generation: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state
            .latest_observations
            .get(&cache_key_hash(key))
            .is_some_and(|latest| *latest == generation)
        {
            remove_entry(&mut state, key);
        }
    }

    pub(super) fn get(
        &self,
        key: &ScrapeCacheKey,
        formats: &[ScrapeFormat],
        max_age: Duration,
    ) -> Option<Arc<ScrapeDocument>> {
        if max_age.is_zero() {
            return None;
        }
        let mut state = self.state.lock().ok()?;
        let requested = formats_mask(formats);
        let now = Instant::now();
        let usable = state.entries.get(key).is_some_and(|entry| {
            now <= entry.expires_at
                && now.duration_since(entry.observed_at) <= max_age.min(CACHE_TTL)
                && entry.formats & requested == requested
        });
        if !usable {
            if state
                .entries
                .get(key)
                .is_some_and(|entry| now > entry.expires_at)
            {
                remove_entry(&mut state, key);
            }
            return None;
        }
        let document = Arc::clone(&state.entries.get(key)?.document);
        touch(&mut state.order, key);
        Some(document)
    }

    pub(super) fn insert(
        &self,
        key: ScrapeCacheKey,
        generation: u64,
        observed_at: Instant,
        ttl: Duration,
        document: ScrapeDocument,
        formats: &[ScrapeFormat],
    ) -> bool {
        let size = entry_size(&key, &document);
        if size > MAX_CACHE_ENTRY_BYTES || ttl.is_zero() {
            return false;
        }
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.latest_observations.get(&cache_key_hash(&key)) != Some(&generation) {
            return false;
        }
        insert_entry(
            &mut state,
            key,
            CacheEntry {
                document: Arc::new(document),
                formats: formats_mask(formats),
                observed_at,
                expires_at: observed_at + ttl.min(CACHE_TTL),
                size,
                generation,
            },
        );
        true
    }

    fn insert_warmed(&self, key: ScrapeCacheKey, generation: u64, document: ScrapeDocument) {
        let size = entry_size(&key, &document);
        if size > MAX_CACHE_ENTRY_BYTES {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let Some(current) = state.entries.get(&key) else {
            return;
        };
        if state.latest_observations.get(&cache_key_hash(&key)) != Some(&generation)
            || current.generation != generation
            || Instant::now() > current.expires_at
        {
            return;
        }
        let observed_at = current.observed_at;
        let expires_at = current.expires_at;
        insert_entry(
            &mut state,
            key,
            CacheEntry {
                document: Arc::new(document),
                formats: ALL_FORMATS_MASK,
                observed_at,
                expires_at,
                size,
                generation,
            },
        );
    }
}

pub(super) struct CacheWarmJob {
    pub key: ScrapeCacheKey,
    pub generation: u64,
    pub input: ExtractInput,
    pub options: ScrapeOptions,
    reserved_bytes: usize,
}

pub(super) struct CacheWarmer {
    sender: mpsc::Sender<CacheWarmJob>,
    retained_bytes: Arc<AtomicUsize>,
}

impl CacheWarmer {
    pub(super) fn try_send(&self, mut job: CacheWarmJob) -> bool {
        let bytes = job.input.html.len();
        let reserved =
            self.retained_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current
                        .checked_add(bytes)
                        .filter(|next| *next <= MAX_CACHE_WARM_BYTES)
                });
        if reserved.is_err() {
            return false;
        }
        job.reserved_bytes = bytes;
        if self.sender.try_send(job).is_err() {
            self.retained_bytes.fetch_sub(bytes, Ordering::AcqRel);
            return false;
        }
        true
    }
}

pub(super) fn warm_job(
    key: ScrapeCacheKey,
    generation: u64,
    input: ExtractInput,
    options: ScrapeOptions,
) -> CacheWarmJob {
    CacheWarmJob {
        key,
        generation,
        input,
        options,
        reserved_bytes: 0,
    }
}

pub(super) fn start_warmer(cache: Arc<ScrapeCache>) -> CacheWarmer {
    let (sender, mut receiver) = mpsc::channel::<CacheWarmJob>(CACHE_WARM_QUEUE_CAPACITY);
    let retained_bytes = Arc::new(AtomicUsize::new(0));
    let worker_retained_bytes = Arc::clone(&retained_bytes);
    tokio::spawn(async move {
        while let Some(mut job) = receiver.recv().await {
            // Let the foreground task serialize and flush its requested formats
            // before spending CPU on speculative extraction.
            tokio::time::sleep(Duration::from_millis(25)).await;
            job.options.formats = ALL_FORMATS.to_vec();
            let reserved_bytes = job.reserved_bytes;
            let extraction = tokio::task::spawn_blocking(move || {
                let document = extract::extract(job.input, &job.options);
                (job.key, job.generation, document)
            })
            .await;
            worker_retained_bytes.fetch_sub(reserved_bytes, Ordering::AcqRel);
            match extraction {
                Ok((key, generation, Ok(mut document))) => {
                    document.elapsed_ms = 0;
                    cache.insert_warmed(key, generation, document);
                    debug!(operation = "scrape", "background format cache warmed");
                }
                Ok((_, _, Err(error))) => debug!(
                    operation = "scrape",
                    %error,
                    "background format extraction failed"
                ),
                Err(error) => debug!(
                    operation = "scrape",
                    %error,
                    "background format worker failed"
                ),
            }
        }
    });
    CacheWarmer {
        sender,
        retained_bytes,
    }
}

pub(super) fn needs_warm(formats: &[ScrapeFormat]) -> bool {
    formats_mask(formats) != ALL_FORMATS_MASK
}

pub(super) fn project_document(
    cached: &ScrapeDocument,
    requested_url: &str,
    formats: &[ScrapeFormat],
    elapsed_ms: u64,
) -> ScrapeDocument {
    let wants = |format| formats.contains(&format);
    let mut metadata = cached.metadata.clone();
    if !wants(ScrapeFormat::Metadata) {
        metadata.headers.clear();
    }
    let wants_content =
        wants(ScrapeFormat::Markdown) || wants(ScrapeFormat::Html) || wants(ScrapeFormat::Text);
    let warnings = cached
        .warnings
        .iter()
        .filter(|warning| {
            (wants_content
                || warning.as_str() != "main-content extraction fell back to the full document")
                && (wants(ScrapeFormat::Markdown)
                    || warning.as_str() != "extracted Markdown is empty")
        })
        .cloned()
        .collect();
    ScrapeDocument {
        url: requested_url.to_owned(),
        final_url: cached.final_url.clone(),
        engine: cached.engine,
        elapsed_ms,
        metadata,
        markdown: if wants(ScrapeFormat::Markdown) {
            cached.markdown.clone()
        } else {
            None
        },
        html: if wants(ScrapeFormat::Html) {
            cached.html.clone()
        } else {
            None
        },
        text: if wants(ScrapeFormat::Text) {
            cached.text.clone()
        } else {
            None
        },
        links: if wants(ScrapeFormat::Links) {
            cached.links.clone()
        } else {
            None
        },
        warnings,
    }
}

fn cache_key_hash(key: &ScrapeCacheKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

fn insert_entry(state: &mut CacheState, key: ScrapeCacheKey, entry: CacheEntry) {
    if let Some(previous) = state.entries.remove(&key) {
        state.bytes = state.bytes.saturating_sub(previous.size);
    }
    state.bytes = state.bytes.saturating_add(entry.size);
    state.entries.insert(key.clone(), entry);
    touch(&mut state.order, &key);

    let now = Instant::now();
    let expired = state
        .entries
        .iter()
        .filter(|(_, entry)| now > entry.expires_at)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in expired {
        remove_entry(state, &key);
    }
    while state.entries.len() > MAX_CACHE_ENTRIES || state.bytes > MAX_CACHE_BYTES {
        let Some(oldest) = state.order.pop_front() else {
            break;
        };
        if let Some(entry) = state.entries.remove(&oldest) {
            state.bytes = state.bytes.saturating_sub(entry.size);
        }
    }
}

fn remove_entry(state: &mut CacheState, key: &ScrapeCacheKey) {
    if let Some(entry) = state.entries.remove(key) {
        state.bytes = state.bytes.saturating_sub(entry.size);
    }
    state.order.retain(|candidate| candidate != key);
}

fn touch(order: &mut VecDeque<ScrapeCacheKey>, key: &ScrapeCacheKey) {
    order.retain(|candidate| candidate != key);
    order.push_back(key.clone());
}

fn formats_mask(formats: &[ScrapeFormat]) -> u8 {
    formats.iter().fold(0, |mask, format| {
        mask | match format {
            ScrapeFormat::Markdown => 1,
            ScrapeFormat::Html => 1 << 1,
            ScrapeFormat::Text => 1 << 2,
            ScrapeFormat::Links => 1 << 3,
            ScrapeFormat::Metadata => 1 << 4,
        }
    })
}

fn entry_size(key: &ScrapeCacheKey, document: &ScrapeDocument) -> usize {
    // The key is owned once by the map and once by the LRU queue. Fixed
    // overhead covers the hash-table bucket, Arc allocation, and tree nodes.
    document_size(document)
        .saturating_add(key.url.capacity().saturating_mul(2))
        .saturating_add(size_of::<ScrapeCacheKey>().saturating_mul(2))
        .saturating_add(512)
}

fn document_size(document: &ScrapeDocument) -> usize {
    let string = |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
    let links = document.links.as_ref().map_or(0, |links| {
        links
            .capacity()
            .saturating_mul(size_of::<scorch_types::Link>())
            + links
                .iter()
                .map(|link| link.url.capacity() + string(&link.text))
                .sum::<usize>()
    });
    let warnings = document
        .warnings
        .capacity()
        .saturating_mul(size_of::<String>())
        + document
            .warnings
            .iter()
            .map(String::capacity)
            .sum::<usize>();
    let headers = document
        .metadata
        .headers
        .iter()
        .map(|(name, value)| {
            size_of::<(String, String)>() + name.capacity() + value.capacity() + 32
        })
        .sum::<usize>();

    size_of::<ScrapeDocument>()
        + document.url.capacity()
        + document.final_url.capacity()
        + string(&document.metadata.title)
        + string(&document.metadata.description)
        + string(&document.metadata.language)
        + string(&document.metadata.canonical_url)
        + string(&document.metadata.content_type)
        + headers
        + string(&document.markdown)
        + string(&document.html)
        + string(&document.text)
        + links
        + warnings
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use scorch_types::{PageMetadata, ScrapeEngine};

    use super::*;

    fn document() -> ScrapeDocument {
        ScrapeDocument {
            url: "https://example.com".into(),
            final_url: "https://example.com/".into(),
            engine: ScrapeEngine::Obscura,
            elapsed_ms: 12,
            metadata: PageMetadata {
                title: Some("Example".into()),
                description: None,
                language: None,
                canonical_url: None,
                status_code: 200,
                content_type: Some("text/html".into()),
                headers: BTreeMap::from([("etag".into(), "abc".into())]),
            },
            markdown: Some("# Example".into()),
            html: Some("<h1>Example</h1>".into()),
            text: Some("Example".into()),
            links: Some(Vec::new()),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn cache_serves_only_covered_formats() {
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let key = ScrapeCacheKey::new("https://example.com#fragment", &options);
        cache.insert(
            key.clone(),
            cache.begin_observation(&key),
            Instant::now(),
            Duration::from_secs(60),
            document(),
            &[ScrapeFormat::Markdown],
        );

        assert!(
            cache
                .get(&key, &[ScrapeFormat::Markdown], Duration::from_secs(1))
                .is_some()
        );
        assert!(
            cache
                .get(&key, &[ScrapeFormat::Html], Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn zero_max_age_bypasses_cache() {
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let key = ScrapeCacheKey::new("https://example.com", &options);
        cache.insert(
            key.clone(),
            cache.begin_observation(&key),
            Instant::now(),
            Duration::from_secs(60),
            document(),
            &[ScrapeFormat::Markdown],
        );

        assert!(
            cache
                .get(&key, &[ScrapeFormat::Markdown], Duration::ZERO)
                .is_none()
        );
    }

    #[test]
    fn current_uncacheable_observation_invalidates_older_content() {
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let key = ScrapeCacheKey::new("https://example.com", &options);
        let original = cache.begin_observation(&key);
        assert!(cache.insert(
            key.clone(),
            original,
            Instant::now(),
            Duration::from_secs(60),
            document(),
            &[ScrapeFormat::Markdown],
        ));

        let refresh = cache.begin_observation(&key);
        cache.invalidate_observation(&key, refresh);

        assert!(
            cache
                .get(&key, &[ScrapeFormat::Markdown], Duration::from_secs(60))
                .is_none()
        );
    }

    #[test]
    fn older_observation_cannot_replace_newer_content() {
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let key = ScrapeCacheKey::new("https://example.com", &options);
        let older = cache.begin_observation(&key);
        let newer = cache.begin_observation(&key);
        let mut newest_document = document();
        newest_document.markdown = Some("new".into());
        assert!(cache.insert(
            key.clone(),
            newer,
            Instant::now(),
            Duration::from_secs(60),
            newest_document,
            &[ScrapeFormat::Markdown],
        ));
        let mut old_document = document();
        old_document.markdown = Some("old".into());
        assert!(!cache.insert(
            key.clone(),
            older,
            Instant::now(),
            Duration::from_secs(60),
            old_document,
            &[ScrapeFormat::Markdown],
        ));

        let cached = cache
            .get(&key, &[ScrapeFormat::Markdown], Duration::from_secs(60))
            .unwrap();
        assert_eq!(cached.markdown.as_deref(), Some("new"));
    }

    #[test]
    fn evicted_observation_removes_stale_entry_and_rejects_late_write() {
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let key = ScrapeCacheKey::new("https://example.com", &options);
        let original = cache.begin_observation(&key);
        assert!(cache.insert(
            key.clone(),
            original,
            Instant::now(),
            Duration::from_secs(60),
            document(),
            &[ScrapeFormat::Markdown],
        ));
        let refresh = cache.begin_observation(&key);
        for index in 0..MAX_OBSERVATION_TOMBSTONES {
            let other = ScrapeCacheKey::new(&format!("https://example.com/{index}"), &options);
            cache.begin_observation(&other);
        }

        assert!(
            cache
                .get(&key, &[ScrapeFormat::Markdown], Duration::from_secs(60))
                .is_none()
        );
        assert!(!cache.insert(
            key,
            refresh,
            Instant::now(),
            Duration::from_secs(60),
            document(),
            &[ScrapeFormat::Markdown],
        ));
    }

    #[test]
    fn projection_drops_unrequested_cached_fields() {
        let projected = project_document(
            &document(),
            "https://example.com#new",
            &[ScrapeFormat::Markdown],
            1,
        );

        assert_eq!(projected.url, "https://example.com#new");
        assert_eq!(projected.markdown.as_deref(), Some("# Example"));
        assert!(projected.html.is_none());
        assert!(projected.text.is_none());
        assert!(projected.links.is_none());
        assert!(projected.metadata.headers.is_empty());
        assert_eq!(projected.elapsed_ms, 1);
    }

    #[tokio::test]
    async fn concurrent_identical_misses_share_one_render_outcome() {
        let flights = Arc::new(RenderFlights::default());
        let options = ScrapeOptions::default();
        let key = RenderFlightKey::new("https://example.com", &options);
        let leader = flights.acquire(key.clone()).unwrap();
        let mut follower = flights.acquire(key.clone()).unwrap();
        assert!(leader.is_leader);
        assert!(!follower.is_leader);
        let (producer_key, entry, _) = leader.lease.producer();
        flights.complete(
            &producer_key,
            &entry,
            RenderFlightOutcome::Rendered(RenderedPage {
                html: "<html></html>".into(),
                final_url: "https://example.com/".into(),
                status: 200,
                content_type: Some("text/html".into()),
                headers: BTreeMap::new(),
                cache_ttl: None,
                cache_observed_at: None,
            }),
        );

        let outcome = follower.lease.wait().await.unwrap();
        assert!(matches!(outcome.as_ref(), RenderFlightOutcome::Rendered(_)));
    }

    #[test]
    fn completed_flight_is_retained_only_while_callers_extract() {
        let flights = Arc::new(RenderFlights::default());
        let options = ScrapeOptions::default();
        let key = RenderFlightKey::new("https://example.com", &options);
        let leader = flights.acquire(key.clone()).unwrap();
        let follower = flights.acquire(key.clone()).unwrap();
        let (producer_key, entry, _) = leader.lease.producer();
        flights.complete(
            &producer_key,
            &entry,
            RenderFlightOutcome::Failed(EngineError::Timeout),
        );
        drop(leader);
        assert_eq!(flights.entries.lock().unwrap().len(), 1);
        drop(follower);
        assert!(flights.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn coalesced_callers_share_cache_observation_and_one_publication() {
        let flights = Arc::new(RenderFlights::default());
        let cache = ScrapeCache::default();
        let options = ScrapeOptions::default();
        let cache_key = ScrapeCacheKey::new("https://example.com", &options);
        let key = RenderFlightKey::new("https://example.com", &options);
        let first = flights.acquire(key.clone()).unwrap();
        let second = flights.acquire(key).unwrap();

        let first_generation = first
            .lease
            .prepare_cache_observation(&cache, &cache_key)
            .unwrap();
        let second_generation = second
            .lease
            .prepare_cache_observation(&cache, &cache_key)
            .unwrap();
        assert_eq!(first_generation, second_generation);
        assert!(first.lease.claim_cache_publication(&cache_key));
        assert!(!second.lease.claim_cache_publication(&cache_key));
    }

    #[test]
    fn extraction_variants_share_render_but_publish_separate_cache_entries() {
        let flights = Arc::new(RenderFlights::default());
        let cache = ScrapeCache::default();
        let main_options = ScrapeOptions::default();
        let mut full_options = main_options.clone();
        full_options.only_main_content = false;
        let flight_key = RenderFlightKey::new("https://example.com", &main_options);
        assert_eq!(
            flight_key,
            RenderFlightKey::new("https://example.com", &full_options)
        );
        let main = flights.acquire(flight_key.clone()).unwrap();
        let full = flights.acquire(flight_key).unwrap();
        let main_key = ScrapeCacheKey::new("https://example.com", &main_options);
        let full_key = ScrapeCacheKey::new("https://example.com", &full_options);

        assert_ne!(
            main.lease
                .prepare_cache_observation(&cache, &main_key)
                .unwrap(),
            full.lease
                .prepare_cache_observation(&cache, &full_key)
                .unwrap()
        );
        assert!(main.lease.claim_cache_publication(&main_key));
        assert!(full.lease.claim_cache_publication(&full_key));
    }

    #[tokio::test]
    async fn retry_cache_detaches_existing_waiters_before_new_acquisition() {
        let flights = Arc::new(RenderFlights::default());
        let options = ScrapeOptions::default();
        let key = RenderFlightKey::new("https://example.com", &options);
        let leader = flights.acquire(key.clone()).unwrap();
        let mut follower = flights.acquire(key.clone()).unwrap();
        let (producer_key, entry, _) = leader.lease.producer();
        flights.retry_cache(&producer_key, &entry);

        assert!(matches!(
            follower.lease.wait().await.unwrap().as_ref(),
            RenderFlightOutcome::RetryCache
        ));
        assert!(flights.acquire(key).unwrap().is_leader);
    }

    #[test]
    fn dropping_last_incomplete_lease_cancels_producer() {
        let flights = Arc::new(RenderFlights::default());
        let options = ScrapeOptions::default();
        let key = RenderFlightKey::new("https://example.com", &options);
        let leader = flights.acquire(key).unwrap();
        let (_, _, cancellation) = leader.lease.producer();

        drop(leader);
        assert!(cancellation.is_cancelled());
    }
}
