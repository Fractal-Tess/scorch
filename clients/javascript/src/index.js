const DEFAULT_API_URL = "http://127.0.0.1:33000";
const DEFAULT_TIMEOUT_MS = 135_000;
const DEFAULT_MAX_RESPONSE_BYTES = 64 * 1024 * 1024;
const MAX_TIMEOUT_MS = 2_147_483_647;

export class ScorchError extends Error {
  constructor(message, options) {
    super(message, options);
    this.name = new.target.name;
  }
}

export class ScorchAPIError extends ScorchError {
  constructor(message, { status, code = "http_error", requestId } = {}) {
    super(`${code}: ${message}`);
    this.message = message;
    this.status = status;
    this.code = code;
    this.requestId = requestId;
  }
}

export class ScorchConnectionError extends ScorchError {}
export class ScorchResponseError extends ScorchError {}

export class ScorchClient {
  constructor({
    baseUrl = environmentApiUrl() ?? DEFAULT_API_URL,
    timeoutMs = DEFAULT_TIMEOUT_MS,
    headers,
    maxResponseBytes = DEFAULT_MAX_RESPONSE_BYTES,
    fetch: fetchImplementation = globalThis.fetch,
  } = {}) {
    this.baseUrl = validateBaseUrl(baseUrl);
    if (
      !Number.isFinite(timeoutMs) ||
      timeoutMs <= 0 ||
      timeoutMs > MAX_TIMEOUT_MS
    ) {
      throw new TypeError(
        `timeoutMs must be greater than zero and no more than ${MAX_TIMEOUT_MS}`,
      );
    }
    if (!Number.isSafeInteger(maxResponseBytes) || maxResponseBytes <= 0) {
      throw new TypeError("maxResponseBytes must be a positive safe integer");
    }
    if (typeof fetchImplementation !== "function") {
      throw new TypeError("A Fetch API implementation is required");
    }

    this.timeoutMs = timeoutMs;
    this.maxResponseBytes = maxResponseBytes;
    this.fetch = fetchImplementation;
    this.headers = new Headers(headers);
    if (!this.headers.has("accept")) {
      this.headers.set("accept", "application/json");
    }
  }

  health(options) {
    return this.#request("GET", "/health", undefined, options?.signal);
  }

  readiness(options) {
    return this.#request("GET", "/ready", undefined, options?.signal);
  }

  scrape(url, options, requestOptions) {
    const body = { url };
    if (options !== undefined) body.options = options;
    return this.#request("POST", "/v1/scrape", body, requestOptions?.signal);
  }

  search(query, options = {}, requestOptions) {
    return this.#request(
      "POST",
      "/v1/search",
      compact({ query, ...options }),
      requestOptions?.signal,
    );
  }

  map(url, options = {}, requestOptions) {
    return this.#request(
      "POST",
      "/v1/map",
      compact({ url, ...options }),
      requestOptions?.signal,
    );
  }

  startCrawl(url, options = {}, requestOptions) {
    return this.#request(
      "POST",
      "/v1/crawls",
      compact({ url, ...options }),
      requestOptions?.signal,
    );
  }

  crawlStatus(crawlId, options = {}, requestOptions) {
    const query = new URLSearchParams({
      cursor: String(options.cursor ?? 0),
      pageSize: String(options.pageSize ?? 10),
    });
    return this.#request(
      "GET",
      `/v1/crawls/${encodeURIComponent(crawlId)}?${query}`,
      undefined,
      requestOptions?.signal,
    );
  }

  cancelCrawl(crawlId, requestOptions) {
    return this.#request(
      "DELETE",
      `/v1/crawls/${encodeURIComponent(crawlId)}`,
      undefined,
      requestOptions?.signal,
    );
  }

  async #request(method, path, body, callerSignal) {
    const headers = new Headers(this.headers);
    let encodedBody;
    if (body !== undefined) {
      headers.set("content-type", "application/json");
      encodedBody = JSON.stringify(body);
    }

    const controller = new AbortController();
    let timedOut = false;
    const timeout = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, this.timeoutMs);
    const abort = () => controller.abort(callerSignal?.reason);
    callerSignal?.addEventListener("abort", abort, { once: true });
    if (callerSignal?.aborted) abort();

    let response;
    try {
      response = await this.fetch(`${this.baseUrl}${path}`, {
        method,
        headers,
        body: encodedBody,
        signal: controller.signal,
        redirect: "error",
      });
    } catch (error) {
      clearTimeout(timeout);
      callerSignal?.removeEventListener("abort", abort);
      const reason = timedOut
        ? `Scorch API request timed out after ${this.timeoutMs} ms`
        : callerSignal?.aborted
          ? "Scorch API request was aborted"
          : `Scorch API request failed: ${errorMessage(error)}`;
      throw new ScorchConnectionError(reason, { cause: error });
    }

    try {
      return await parseResponse(response, this.maxResponseBytes);
    } catch (error) {
      if (timedOut) {
        throw new ScorchConnectionError(
          `Scorch API request timed out after ${this.timeoutMs} ms`,
          { cause: error },
        );
      }
      if (callerSignal?.aborted) {
        throw new ScorchConnectionError("Scorch API request was aborted", {
          cause: error,
        });
      }
      if (error instanceof ScorchError) throw error;
      throw new ScorchConnectionError(
        `Scorch API response failed: ${errorMessage(error)}`,
        { cause: error },
      );
    } finally {
      clearTimeout(timeout);
      callerSignal?.removeEventListener("abort", abort);
    }
  }
}

async function parseResponse(response, limit) {
  const bytes = await readLimited(response, limit);
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch (error) {
    throw new ScorchResponseError("Scorch API returned invalid UTF-8", {
      cause: error,
    });
  }
  let payload;
  try {
    payload = JSON.parse(text);
  } catch (error) {
    if (!response.ok) {
      throw new ScorchAPIError(text || response.statusText, {
        status: response.status,
        requestId: response.headers.get("x-request-id") ?? undefined,
      });
    }
    throw new ScorchResponseError("Scorch API returned invalid JSON", {
      cause: error,
    });
  }

  if (!response.ok) {
    const structured = isRecord(payload) ? payload : {};
    throw new ScorchAPIError(
      typeof structured.message === "string"
        ? structured.message
        : `Scorch API returned HTTP ${response.status}`,
      {
        status: response.status,
        code:
          typeof structured.code === "string" ? structured.code : "http_error",
        requestId:
          typeof structured.requestId === "string"
            ? structured.requestId
            : (response.headers.get("x-request-id") ?? undefined),
      },
    );
  }

  return payload;
}

async function readLimited(response, limit) {
  let expectedLength;
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    const parsedLength = Number(contentLength);
    if (Number.isSafeInteger(parsedLength) && parsedLength >= 0) {
      expectedLength = parsedLength;
    }
    if (expectedLength !== undefined && expectedLength > limit) {
      const error = new ScorchResponseError(
        `Scorch API response exceeds the ${limit} byte limit`,
      );
      try {
        await response.body?.cancel();
      } catch {
        // Preserve the deterministic response-limit error.
      }
      throw error;
    }
  }

  if (response.body === null) {
    if (expectedLength !== undefined && expectedLength !== 0) {
      throw new ScorchResponseError("Scorch API returned a truncated response");
    }
    return new Uint8Array();
  }
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) {
        await reader.cancel();
        throw new ScorchResponseError(
          `Scorch API response exceeds the ${limit} byte limit`,
        );
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  if (expectedLength !== undefined && size !== expectedLength) {
    throw new ScorchResponseError("Scorch API returned a truncated response");
  }

  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function validateBaseUrl(baseUrl) {
  let parsed;
  try {
    parsed = new URL(baseUrl);
  } catch (error) {
    throw new TypeError("baseUrl must be a valid URL", { cause: error });
  }
  if (!["http:", "https:"].includes(parsed.protocol) || !parsed.hostname) {
    throw new TypeError("baseUrl must use HTTP or HTTPS and include a host");
  }
  if (parsed.username || parsed.password) {
    throw new TypeError(
      "Credentials in baseUrl are not supported; use headers instead",
    );
  }
  if (baseUrl.includes("?") || baseUrl.includes("#")) {
    throw new TypeError("baseUrl cannot contain a query or fragment");
  }
  return baseUrl.replace(/\/+$/, "");
}

function compact(value) {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  );
}

function environmentApiUrl() {
  return typeof process !== "undefined"
    ? process.env?.SCORCH_API_URL
    : undefined;
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export { DEFAULT_API_URL, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS };
