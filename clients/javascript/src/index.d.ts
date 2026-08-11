export const DEFAULT_API_URL: "http://127.0.0.1:33000";
export const DEFAULT_TIMEOUT_MS: 135000;
export const DEFAULT_MAX_RESPONSE_BYTES: 67108864;

export type ScrapeFormat =
  "markdown" | "html" | "text" | "links" | "metadata" | "screenshot";
export type RenderMode = "auto" | "always" | "never";
export type ScrapeEngine = "fetch" | "obscura";
export type SearchEngine =
  "bing" | "brave" | "duckduckgo" | "google" | "naver" | "wikipedia";
export type CrawlStatus =
  "queued" | "running" | "completed" | "cancelled" | "failed";

export interface ScrapeOptions {
  formats?: readonly ScrapeFormat[];
  render?: RenderMode;
  timeoutMs?: number;
  waitForMs?: number;
  onlyMainContent?: boolean;
  blockMedia?: boolean;
  fullPageScreenshot?: boolean;
}

export interface Link {
  url: string;
  text?: string;
}

export interface PageMetadata {
  title?: string;
  description?: string;
  language?: string;
  canonicalUrl?: string;
  statusCode: number;
  contentType?: string;
  headers?: Record<string, string>;
}

export interface ScrapeDocument {
  url: string;
  finalUrl: string;
  engine: ScrapeEngine;
  elapsedMs: number;
  metadata: PageMetadata;
  markdown?: string;
  html?: string;
  text?: string;
  links?: Link[];
  screenshot?: string;
  warnings?: string[];
}

export interface SearchOptions {
  limit?: number;
  country?: string;
  language?: string;
  engines?: readonly SearchEngine[];
  scrapeOptions?: ScrapeOptions | null;
}

export interface SearchResult {
  position: number;
  title: string;
  url: string;
  description?: string;
  sources?: string[];
  document?: ScrapeDocument;
  error?: string;
}

export interface SearchResponse {
  query: string;
  provider: string;
  engines?: string[];
  results: SearchResult[];
  elapsedMs: number;
  warnings?: string[];
}

export interface MapOptions {
  limit?: number;
  includeSubdomains?: boolean;
  includePaths?: readonly string[];
  excludePaths?: readonly string[];
}

export interface MapResponse {
  url: string;
  links: string[];
  elapsedMs: number;
  sources: string[];
}

export interface StartCrawlOptions {
  limit?: number;
  maxDepth?: number;
  concurrency?: number;
  includePaths?: readonly string[];
  excludePaths?: readonly string[];
  scrapeOptions?: ScrapeOptions;
}

export interface CrawlError {
  url: string;
  message: string;
}

export interface CrawlJobSummary {
  id: string;
  status: CrawlStatus;
  createdAtMs: number;
  expiresAtMs: number;
  total: number;
  completed: number;
  errorCount: number;
}

export interface CrawlPage extends CrawlJobSummary {
  cursor: number;
  documents: ScrapeDocument[];
  errors: CrawlError[];
  nextCursor?: number;
}

export interface CrawlStatusOptions {
  cursor?: number;
  pageSize?: number;
}

export interface DeleteResponse {
  id: string;
  deleted: boolean;
}

export interface HealthResponse {
  status: string;
}

export interface ReadinessResponse {
  status: string;
  browserAvailable: boolean;
  browser: string;
  obscuraStealth: boolean;
  maxConcurrency: number;
  searchProvider: string;
  searchEngines: string[];
}

export interface RequestOptions {
  signal?: AbortSignal;
}

export interface ScorchClientOptions {
  baseUrl?: string;
  timeoutMs?: number;
  headers?: HeadersInit;
  maxResponseBytes?: number;
  fetch?: typeof globalThis.fetch;
}

export class ScorchError extends Error {
  constructor(message: string, options?: ErrorOptions);
}

export class ScorchAPIError extends ScorchError {
  constructor(
    message: string,
    options: {
      status: number;
      code?: string;
      requestId?: string;
    },
  );
  readonly status: number;
  readonly code: string;
  readonly requestId?: string;
}

export class ScorchConnectionError extends ScorchError {}
export class ScorchResponseError extends ScorchError {}

export class ScorchClient {
  constructor(options?: ScorchClientOptions);
  readonly baseUrl: string;
  readonly timeoutMs: number;
  readonly maxResponseBytes: number;

  health(options?: RequestOptions): Promise<HealthResponse>;
  readiness(options?: RequestOptions): Promise<ReadinessResponse>;
  scrape(
    url: string,
    options?: ScrapeOptions,
    requestOptions?: RequestOptions,
  ): Promise<ScrapeDocument>;
  search(
    query: string,
    options?: SearchOptions,
    requestOptions?: RequestOptions,
  ): Promise<SearchResponse>;
  map(
    url: string,
    options?: MapOptions,
    requestOptions?: RequestOptions,
  ): Promise<MapResponse>;
  startCrawl(
    url: string,
    options?: StartCrawlOptions,
    requestOptions?: RequestOptions,
  ): Promise<CrawlJobSummary>;
  crawlStatus(
    crawlId: string,
    options?: CrawlStatusOptions,
    requestOptions?: RequestOptions,
  ): Promise<CrawlPage>;
  cancelCrawl(
    crawlId: string,
    requestOptions?: RequestOptions,
  ): Promise<DeleteResponse>;
}
