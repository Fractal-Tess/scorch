import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { createServer } from "node:http";

import {
  ScorchAPIError,
  ScorchClient,
  ScorchConnectionError,
  ScorchResponseError,
} from "../src/index.js";

let baseUrl;
const server = createServer(async (request, response) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const body = chunks.length
    ? JSON.parse(Buffer.concat(chunks).toString("utf8"))
    : undefined;

  response.setHeader("content-type", "application/json");
  response.setHeader("x-request-id", "test-request");

  if (
    request.method === "GET" &&
    request.url === "/health" &&
    request.headers["x-redirect-test"]
  ) {
    response.statusCode = 302;
    response.setHeader("location", "http://127.0.0.1:9/capture");
    response.setHeader("content-length", "0");
    response.end();
  } else if (request.method === "GET" && request.url === "/health") {
    send(response, 200, { status: "ok" });
  } else if (request.method === "GET" && request.url === "/ready") {
    send(response, 200, {
      status: "ready",
      browserAvailable: true,
      browser: "obscura",
      obscuraStealth: true,
      maxConcurrency: 4,
      searchProvider: "metasearch",
      searchEngines: ["bing", "duckduckgo"],
    });
  } else if (request.method === "POST" && request.url === "/v1/search") {
    if (body.query === "bad") {
      send(response, 400, {
        code: "invalid_request",
        message: "bad query",
      });
    } else {
      send(response, 200, {
        query: body.query,
        provider: "metasearch",
        engines: body.engines ?? ["duckduckgo"],
        results:
          body.categories?.[0] === "github"
            ? [
                {
                  position: 1,
                  title: "Repository",
                  url: "https://github.com/example/repository",
                  category: "github",
                },
              ]
            : [],
        elapsedMs: 1,
      });
    }
  } else if (request.method === "POST" && request.url === "/v1/scrape") {
    send(response, 200, {
      url: body.url,
      finalUrl: body.url,
      engine: "fetch",
      elapsedMs: 1,
      metadata: { statusCode: 200 },
      markdown: "ok",
    });
  } else if (request.method === "POST" && request.url === "/v1/map") {
    send(response, 200, {
      url: body.url,
      links: [],
      elapsedMs: 1,
      sources: [],
    });
  } else if (request.method === "POST" && request.url === "/v1/crawls") {
    send(response, 202, {
      id: "job-id",
      status: "queued",
      createdAtMs: 1,
      expiresAtMs: 2,
      total: 0,
      completed: 0,
      errorCount: 0,
    });
  } else if (
    request.method === "GET" &&
    request.url === "/v1/crawls/job-id?cursor=0&pageSize=10"
  ) {
    send(response, 200, {
      id: "job-id",
      status: "running",
      createdAtMs: 1,
      expiresAtMs: 2,
      total: 1,
      completed: 0,
      errorCount: 0,
      cursor: 0,
      documents: [],
      errors: [],
    });
  } else if (
    request.method === "DELETE" &&
    request.url === "/v1/crawls/job-id"
  ) {
    send(response, 200, { id: "job-id", deleted: true });
  } else {
    send(response, 404, { code: "not_found", message: "missing" });
  }
});

before(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  baseUrl = `http://${address.address}:${address.port}`;
});

after(async () => {
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
});

test("health, readiness, and search", async () => {
  const client = new ScorchClient({ baseUrl });
  assert.equal((await client.health()).status, "ok");
  assert.equal((await client.readiness()).browser, "obscura");
  const result = await client.search("Rust", {
    country: "bg",
    language: "en",
    engines: [
      "brave-web",
      "crates-io",
      "docker-hub",
      "google-cse",
      "npm",
      "yahoo",
    ],
    categories: ["github"],
  });
  assert.deepEqual(result.engines, [
    "brave-web",
    "crates-io",
    "docker-hub",
    "google-cse",
    "npm",
    "yahoo",
  ]);
  assert.equal(result.results[0].category, "github");
});

test("scrape, map, and crawl", async () => {
  const client = new ScorchClient({ baseUrl });
  assert.equal((await client.scrape("https://example.com")).markdown, "ok");
  assert.deepEqual((await client.map("https://example.com")).links, []);
  const job = await client.startCrawl("https://example.com");
  assert.equal((await client.crawlStatus(job.id)).status, "running");
  assert.equal((await client.cancelCrawl(job.id)).deleted, true);
});

test("structured API errors retain request metadata", async () => {
  const client = new ScorchClient({ baseUrl });
  await assert.rejects(client.search("bad"), (error) => {
    assert.ok(error instanceof ScorchAPIError);
    assert.equal(error.status, 400);
    assert.equal(error.code, "invalid_request");
    assert.equal(error.requestId, "test-request");
    return true;
  });
});

test("response bounds and base URL validation", async () => {
  const client = new ScorchClient({ baseUrl, maxResponseBytes: 5 });
  await assert.rejects(client.health(), ScorchResponseError);
  assert.throws(
    () => new ScorchClient({ baseUrl: "https://user:secret@example.com" }),
    TypeError,
  );
  assert.throws(
    () => new ScorchClient({ baseUrl: "file:///tmp/scorch.sock" }),
    TypeError,
  );
  assert.throws(
    () => new ScorchClient({ baseUrl: "https://example.com?" }),
    TypeError,
  );
  assert.throws(
    () => new ScorchClient({ timeoutMs: 2_147_483_648 }),
    TypeError,
  );
  const redirectClient = new ScorchClient({
    baseUrl,
    headers: {
      Authorization: "Bearer secret",
      "x-redirect-test": "true",
    },
  });
  await assert.rejects(redirectClient.health(), ScorchConnectionError);
});

test("serialization failures do not leave a request timer active", async () => {
  const circular = {};
  circular.self = circular;
  const client = new ScorchClient({ baseUrl });
  await assert.rejects(
    client.search("Rust", { scrapeOptions: circular }),
    TypeError,
  );
});

test("an oversized declared response cancels its body", async () => {
  let cancelled = false;
  const oversizedResponse = () =>
    Promise.resolve(
      new Response(
        new ReadableStream({
          cancel() {
            cancelled = true;
          },
        }),
        { status: 200, headers: { "content-length": "100" } },
      ),
    );
  const client = new ScorchClient({
    fetch: oversizedResponse,
    maxResponseBytes: 5,
  });
  await assert.rejects(client.health(), ScorchResponseError);
  assert.equal(cancelled, true);
});

test("body-stream failures are connection errors", async () => {
  const failedResponse = () =>
    Promise.resolve(
      new Response(
        new ReadableStream({
          start(controller) {
            controller.error(new Error("stream failed"));
          },
        }),
        { status: 200 },
      ),
    );
  const client = new ScorchClient({ fetch: failedResponse });
  await assert.rejects(client.health(), ScorchConnectionError);
});

test("invalid UTF-8 is a response error", async () => {
  const invalidResponse = () =>
    Promise.resolve(
      new Response(new Uint8Array([0x7b, 0x22, 0xff, 0x22, 0x7d]), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
  const client = new ScorchClient({ fetch: invalidResponse });
  await assert.rejects(client.health(), ScorchResponseError);
});

test("request timeout aborts the configured fetch", async () => {
  const never = (_url, { signal }) =>
    new Promise((_resolve, reject) => {
      signal.addEventListener(
        "abort",
        () => reject(new DOMException("aborted", "AbortError")),
        { once: true },
      );
    });
  const client = new ScorchClient({ timeoutMs: 5, fetch: never });
  await assert.rejects(client.health(), ScorchConnectionError);
});

function send(response, status, body) {
  const payload = JSON.stringify(body);
  response.statusCode = status;
  response.setHeader("content-length", Buffer.byteLength(payload));
  response.end(payload);
}
