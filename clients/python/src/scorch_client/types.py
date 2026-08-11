from typing import Literal, NotRequired, TypedDict

ScrapeFormat = Literal["markdown", "html", "text", "links", "metadata", "screenshot"]
RenderMode = Literal["auto", "always", "never"]
ScrapeEngine = Literal["fetch", "obscura"]
SearchEngine = Literal["bing", "brave", "duckduckgo", "google", "naver", "wikipedia"]
CrawlStatus = Literal["queued", "running", "completed", "cancelled", "failed"]


class ScrapeOptions(TypedDict, total=False):
    formats: list[ScrapeFormat]
    render: RenderMode
    timeoutMs: int
    waitForMs: int
    onlyMainContent: bool
    blockMedia: bool
    fullPageScreenshot: bool


class Link(TypedDict):
    url: str
    text: NotRequired[str]


class PageMetadata(TypedDict):
    statusCode: int
    title: NotRequired[str]
    description: NotRequired[str]
    language: NotRequired[str]
    canonicalUrl: NotRequired[str]
    contentType: NotRequired[str]
    headers: NotRequired[dict[str, str]]


class ScrapeDocument(TypedDict):
    url: str
    finalUrl: str
    engine: ScrapeEngine
    elapsedMs: int
    metadata: PageMetadata
    markdown: NotRequired[str]
    html: NotRequired[str]
    text: NotRequired[str]
    links: NotRequired[list[Link]]
    screenshot: NotRequired[str]
    warnings: NotRequired[list[str]]


class SearchResult(TypedDict):
    position: int
    title: str
    url: str
    description: NotRequired[str]
    sources: NotRequired[list[str]]
    document: NotRequired[ScrapeDocument]
    error: NotRequired[str]


class SearchResponse(TypedDict):
    query: str
    provider: str
    engines: NotRequired[list[str]]
    results: list[SearchResult]
    elapsedMs: int
    warnings: NotRequired[list[str]]


class MapResponse(TypedDict):
    url: str
    links: list[str]
    elapsedMs: int
    sources: list[str]


class CrawlError(TypedDict):
    url: str
    message: str


class CrawlJobSummary(TypedDict):
    id: str
    status: CrawlStatus
    createdAtMs: int
    expiresAtMs: int
    total: int
    completed: int
    errorCount: int


class CrawlPage(CrawlJobSummary):
    cursor: int
    documents: list[ScrapeDocument]
    errors: list[CrawlError]
    nextCursor: NotRequired[int]


class DeleteResponse(TypedDict):
    id: str
    deleted: bool


class HealthResponse(TypedDict):
    status: str


class ReadinessResponse(TypedDict):
    status: str
    browserAvailable: bool
    browser: str
    obscuraStealth: bool
    maxConcurrency: int
    searchProvider: str
    searchEngines: list[str]
