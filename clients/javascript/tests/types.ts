import {
  ScorchAPIError,
  ScorchClient,
  type ScrapeDocument,
  type SearchResponse,
} from "../src/index.js";

const client = new ScorchClient({ baseUrl: "http://127.0.0.1:33000" });
const search: Promise<SearchResponse> = client.search("Rust", {
  engines: [
    "bing",
    "brave-web",
    "crates-io",
    "docker-hub",
    "duckduckgo",
    "google-cse",
    "hugging-face",
    "npm",
    "pubmed",
    "yahoo",
  ],
  categories: ["github"],
  country: "us",
  language: "en",
});
const scrape: Promise<ScrapeDocument> = client.scrape("https://example.com", {
  formats: ["markdown", "links"],
});
const error: ScorchAPIError = new ScorchAPIError("bad request", {
  status: 400,
  code: "invalid_request",
});

void search;
void scrape;
void error;
