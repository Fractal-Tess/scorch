# Scorch JavaScript client

A zero-runtime-dependency ESM client for the Scorch HTTP API. It works with Node.js 22 or newer and modern runtimes that provide the Fetch API. TypeScript declarations are included.

## Install from this repository

```sh
npm install ./clients/javascript
```

The package is not yet published to npm.

## Use

```js
import { ScorchClient } from "@fractal-tess/scorch";

const client = new ScorchClient(); // http://127.0.0.1:33000

const response = await client.search("Rust HTTP clients", {
  categories: ["github"],
  engines: ["brave-web", "crates-io", "npm", "yahoo"],
  limit: 5,
});

for (const result of response.results) {
  console.log(result.title, result.url);
}

const page = await client.scrape("https://example.com", {
  formats: ["markdown", "links"],
  render: "auto",
});
console.log(page.markdown ?? "");
```

Mapping and crawling use the same client:

```js
const site = await client.map("https://example.com", { limit: 100 });

const job = await client.startCrawl("https://example.com", {
  limit: 20,
  maxDepth: 2,
});
const page = await client.crawlStatus(job.id, { cursor: 0, pageSize: 10 });
await client.cancelCrawl(job.id);
```

Set `SCORCH_API_URL` under Node.js or pass `baseUrl` for another service. Static headers support authenticated gateways without putting credentials in the URL:

```js
const client = new ScorchClient({
  baseUrl: "https://scorch.example.com",
  headers: { Authorization: "Bearer ..." },
  timeoutMs: 135_000,
});
```

Every operation also accepts a final `{ signal }` argument for cancellation:

```js
const controller = new AbortController();
const pending = client.search("Rust", {}, { signal: controller.signal });
controller.abort();
await pending;
```

## Errors and limits

- `ScorchAPIError` exposes `status`, `code`, `message`, and `requestId`.
- `ScorchConnectionError` reports connection, timeout, and cancellation failures.
- `ScorchResponseError` reports invalid JSON and oversized responses.
- Responses are streamed into a bounded 64 MiB buffer by default.
- One absolute timeout covers connection, headers, and body reads.
- API redirects are rejected so gateway credentials cannot be forwarded to another origin.

## Test

```sh
cd clients/javascript
npm install
npm run check
npm test
```
