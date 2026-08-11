from __future__ import annotations

import json
import math
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from email.message import Message
from http.client import (
    HTTPConnection,
    HTTPException,
    HTTPResponse,
    HTTPSConnection,
    IncompleteRead,
)
from queue import Empty, Queue
from socket import SHUT_RDWR, socket
from threading import BoundedSemaphore, Lock, Thread
from time import monotonic
from typing import cast
from urllib.parse import quote, urlencode, urlsplit

from .types import (
    CrawlJobSummary,
    CrawlPage,
    DeleteResponse,
    HealthResponse,
    MapResponse,
    ReadinessResponse,
    ScrapeDocument,
    ScrapeOptions,
    SearchEngine,
    SearchResponse,
)

DEFAULT_API_URL = "http://127.0.0.1:33000"
DEFAULT_TIMEOUT = 135.0
DEFAULT_CONNECT_TIMEOUT = 10.0
DEFAULT_MAX_RESPONSE_BYTES = 64 * 1024 * 1024
_CHUNK_SIZE = 64 * 1024
_MAX_REQUEST_WORKERS = 8
_REQUEST_SLOTS = BoundedSemaphore(_MAX_REQUEST_WORKERS)


@dataclass(frozen=True, slots=True)
class _Outcome:
    value: object | None = None
    error: Exception | None = None


class _ConnectionState:
    def __init__(self) -> None:
        self._lock: Lock = Lock()
        self._cancelled: bool = False
        self._connection: HTTPConnection | None = None
        self._transport: socket | None = None

    def attach_connection(self, connection: HTTPConnection) -> None:
        with self._lock:
            if self._cancelled:
                connection.close()
                raise TimeoutError("request deadline exceeded")
            self._connection = connection

    def attach_transport(self, transport: socket) -> None:
        with self._lock:
            if self._cancelled:
                try:
                    transport.shutdown(SHUT_RDWR)
                except OSError:
                    pass
                if self._connection is not None:
                    self._connection.close()
                raise TimeoutError("request deadline exceeded")
            self._transport = transport

    def cancel(self) -> None:
        with self._lock:
            self._cancelled = True
            if self._transport is not None:
                try:
                    self._transport.shutdown(SHUT_RDWR)
                except OSError:
                    pass
            if self._connection is not None:
                self._connection.close()

    def clear(self) -> None:
        with self._lock:
            self._connection = None
            self._transport = None


class ScorchError(Exception):
    """Base exception for Scorch client failures."""


class ScorchAPIError(ScorchError):
    """A structured error returned by scorchd."""

    def __init__(
        self,
        message: str,
        *,
        status: int,
        code: str = "http_error",
        request_id: str | None = None,
    ) -> None:
        super().__init__(f"{code}: {message}")
        self.message: str = message
        self.status: int = status
        self.code: str = code
        self.request_id: str | None = request_id


class ScorchConnectionError(ScorchError):
    """The client could not reach scorchd."""


class ScorchResponseError(ScorchError):
    """scorchd returned an invalid or oversized response."""


class ScorchClient:
    """Synchronous, dependency-free client for the Scorch HTTP API."""

    def __init__(
        self,
        base_url: str | None = None,
        *,
        timeout: float = DEFAULT_TIMEOUT,
        headers: Mapping[str, str] | None = None,
        max_response_bytes: int = DEFAULT_MAX_RESPONSE_BYTES,
    ) -> None:
        configured_url = (
            base_url
            if base_url is not None
            else os.environ.get("SCORCH_API_URL", DEFAULT_API_URL)
        )
        self._base_url: str = _validate_base_url(configured_url)
        if not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be finite and greater than zero")
        max_response_bytes = _validate_max_response_bytes(max_response_bytes)
        self._timeout: float = timeout
        self._max_response_bytes: int = max_response_bytes
        self._headers: dict[str, str] = {
            "Accept": "application/json",
            **(headers or {}),
        }

    @property
    def base_url(self) -> str:
        return self._base_url

    def health(self) -> HealthResponse:
        return cast(HealthResponse, self._request("GET", "/health"))

    def readiness(self) -> ReadinessResponse:
        return cast(ReadinessResponse, self._request("GET", "/ready"))

    def scrape(
        self,
        url: str,
        *,
        options: ScrapeOptions | None = None,
    ) -> ScrapeDocument:
        body: dict[str, object] = {"url": url}
        if options is not None:
            body["options"] = options
        return cast(ScrapeDocument, self._request("POST", "/v1/scrape", body))

    def search(
        self,
        query: str,
        *,
        limit: int | None = None,
        country: str | None = None,
        language: str | None = None,
        engines: Sequence[SearchEngine] | None = None,
        scrape_options: ScrapeOptions | None = None,
    ) -> SearchResponse:
        body: dict[str, object] = {"query": query}
        _set_if_not_none(body, "limit", limit)
        _set_if_not_none(body, "country", country)
        _set_if_not_none(body, "language", language)
        if engines is not None:
            body["engines"] = list(engines)
        _set_if_not_none(body, "scrapeOptions", scrape_options)
        return cast(SearchResponse, self._request("POST", "/v1/search", body))

    def map(
        self,
        url: str,
        *,
        limit: int | None = None,
        include_subdomains: bool | None = None,
        include_paths: Sequence[str] | None = None,
        exclude_paths: Sequence[str] | None = None,
    ) -> MapResponse:
        body: dict[str, object] = {"url": url}
        _set_if_not_none(body, "limit", limit)
        _set_if_not_none(body, "includeSubdomains", include_subdomains)
        if include_paths is not None:
            body["includePaths"] = list(include_paths)
        if exclude_paths is not None:
            body["excludePaths"] = list(exclude_paths)
        return cast(MapResponse, self._request("POST", "/v1/map", body))

    def start_crawl(
        self,
        url: str,
        *,
        limit: int | None = None,
        max_depth: int | None = None,
        concurrency: int | None = None,
        include_paths: Sequence[str] | None = None,
        exclude_paths: Sequence[str] | None = None,
        scrape_options: ScrapeOptions | None = None,
    ) -> CrawlJobSummary:
        body: dict[str, object] = {"url": url}
        _set_if_not_none(body, "limit", limit)
        _set_if_not_none(body, "maxDepth", max_depth)
        _set_if_not_none(body, "concurrency", concurrency)
        if include_paths is not None:
            body["includePaths"] = list(include_paths)
        if exclude_paths is not None:
            body["excludePaths"] = list(exclude_paths)
        _set_if_not_none(body, "scrapeOptions", scrape_options)
        return cast(CrawlJobSummary, self._request("POST", "/v1/crawls", body))

    def crawl_status(
        self,
        crawl_id: str,
        *,
        cursor: int = 0,
        page_size: int = 10,
    ) -> CrawlPage:
        query = urlencode({"cursor": cursor, "pageSize": page_size})
        path = f"/v1/crawls/{quote(crawl_id, safe='')}?{query}"
        return cast(CrawlPage, self._request("GET", path))

    def cancel_crawl(self, crawl_id: str) -> DeleteResponse:
        path = f"/v1/crawls/{quote(crawl_id, safe='')}"
        return cast(DeleteResponse, self._request("DELETE", path))

    def _request(
        self,
        method: str,
        path: str,
        body: Mapping[str, object] | None = None,
    ) -> object:
        deadline = monotonic() + self._timeout
        state = _ConnectionState()
        outcomes: Queue[_Outcome] = Queue(maxsize=1)

        def run() -> None:
            try:
                try:
                    outcome = _Outcome(
                        value=self._request_blocking(
                            method,
                            path,
                            body,
                            deadline=deadline,
                            state=state,
                        )
                    )
                except Exception as error:  # noqa: BLE001 - relay worker exceptions
                    outcome = _Outcome(error=error)
                outcomes.put(outcome)
            finally:
                _REQUEST_SLOTS.release()

        try:
            acquired = _REQUEST_SLOTS.acquire(timeout=_remaining(deadline))
        except TimeoutError as error:
            raise ScorchConnectionError(
                f"Scorch API request timed out after {self._timeout:g} seconds"
            ) from error
        if not acquired:
            raise ScorchConnectionError(
                f"Scorch API request timed out after {self._timeout:g} seconds"
            )

        worker = Thread(target=run, name="scorch-client-request", daemon=True)
        try:
            worker.start()
        except BaseException:
            _REQUEST_SLOTS.release()
            raise

        try:
            outcome = outcomes.get(timeout=_remaining(deadline))
            _ = _remaining(deadline)
        except (Empty, TimeoutError) as error:
            state.cancel()
            raise ScorchConnectionError(
                f"Scorch API request timed out after {self._timeout:g} seconds"
            ) from error
        except BaseException:
            state.cancel()
            raise

        if outcome.error is not None:
            raise outcome.error
        return outcome.value

    def _request_blocking(
        self,
        method: str,
        path: str,
        body: Mapping[str, object] | None,
        *,
        deadline: float,
        state: _ConnectionState,
    ) -> object:
        data = None
        headers = dict(self._headers)
        if body is not None:
            data = json.dumps(body, separators=(",", ":")).encode()
            headers["Content-Type"] = "application/json"

        parsed = urlsplit(f"{self._base_url}{path}")
        host = parsed.hostname
        if host is None:
            raise ScorchConnectionError("Scorch API URL does not include a host")
        target = parsed.path or "/"
        if parsed.query:
            target = f"{target}?{parsed.query}"

        connection_type = (
            HTTPSConnection if parsed.scheme == "https" else HTTPConnection
        )
        connection = connection_type(
            host,
            parsed.port,
            timeout=min(DEFAULT_CONNECT_TIMEOUT, _remaining(deadline)),
        )
        response: HTTPResponse | None = None
        try:
            state.attach_connection(connection)
            connection.connect()
            transport = connection.sock
            if transport is None:
                raise OSError("connection did not create a socket")
            state.attach_transport(transport)
            _set_socket_timeout(transport, deadline)
            connection.request(method, target, body=data, headers=headers)
            _set_socket_timeout(transport, deadline)
            response = connection.getresponse()
            transport = _response_transport(response)
            payload = _read_limited(
                response,
                self._max_response_bytes,
                deadline=deadline,
                transport=transport,
            )
            if not 200 <= response.status < 300:
                _raise_api_error(response.status, response.headers, payload)
        except ScorchError:
            raise
        except (HTTPException, TimeoutError, OSError) as error:
            raise ScorchConnectionError(
                f"Scorch API request failed: {error}"
            ) from error
        finally:
            if response is not None:
                response.close()
            connection.close()
            state.clear()

        try:
            return cast(object, json.loads(payload))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ScorchResponseError("Scorch API returned invalid JSON") from error


def _validate_max_response_bytes(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError("max_response_bytes must be a positive integer")
    return value


def _validate_base_url(base_url: str) -> str:
    normalized = base_url.rstrip("/")
    parsed = urlsplit(normalized)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise ValueError("base_url must use HTTP or HTTPS and include a host")
    try:
        _ = parsed.port
    except ValueError as error:
        raise ValueError("base_url contains an invalid port") from error
    if parsed.username is not None or parsed.password is not None:
        raise ValueError(
            "credentials in base_url are not supported; use headers instead"
        )
    if "?" in normalized or "#" in normalized:
        raise ValueError("base_url cannot contain a query or fragment")
    return normalized


def _set_if_not_none(body: dict[str, object], key: str, value: object | None) -> None:
    if value is not None:
        body[key] = value


def _remaining(deadline: float) -> float:
    remaining = deadline - monotonic()
    if remaining <= 0:
        raise TimeoutError("request deadline exceeded")
    return remaining


def _set_socket_timeout(transport: socket, deadline: float) -> None:
    remaining = _remaining(deadline)
    try:
        _ = transport.settimeout(remaining)
    except OSError:
        if transport.fileno() != -1:
            raise


def _response_transport(response: HTTPResponse) -> socket:
    candidate = cast(object, getattr(response.fp.raw, "_sock", None))
    if not isinstance(candidate, socket):
        raise OSError("response does not have an open transport")
    return candidate


def _read_limited(
    response: HTTPResponse,
    limit: int,
    *,
    deadline: float,
    transport: socket,
) -> bytes:
    expected_length: int | None = None
    content_length = response.headers.get("Content-Length")
    if content_length is not None:
        try:
            expected_length = int(content_length)
        except ValueError:
            expected_length = None
        if expected_length is not None and expected_length > limit:
            raise ScorchResponseError(
                f"Scorch API response exceeds the {limit} byte limit"
            )

    chunks: list[bytes] = []
    size = 0
    while True:
        _set_socket_timeout(transport, deadline)
        try:
            chunk = response.read1(_CHUNK_SIZE)
        except IncompleteRead as error:
            raise ScorchResponseError(
                "Scorch API returned a truncated response"
            ) from error
        if not chunk:
            break
        size += len(chunk)
        if size > limit:
            raise ScorchResponseError(
                f"Scorch API response exceeds the {limit} byte limit"
            )
        chunks.append(chunk)
        if expected_length is not None and size == expected_length:
            break

    if expected_length is not None and size != expected_length:
        raise ScorchResponseError("Scorch API returned a truncated response")
    return b"".join(chunks)


def _raise_api_error(status: int, headers: Message[str, str], payload: bytes) -> None:
    request_id = headers.get("x-request-id")
    try:
        error = cast(object, json.loads(payload))
    except (UnicodeDecodeError, json.JSONDecodeError):
        message = payload.decode(errors="replace")
        raise ScorchAPIError(message, status=status, request_id=request_id)

    if isinstance(error, dict):
        fields = cast(dict[str, object], error)
        message = str(fields.get("message", f"Scorch API returned HTTP {status}"))
        code = str(fields.get("code", "http_error"))
        body_request_id = fields.get("requestId")
        if isinstance(body_request_id, str):
            request_id = body_request_id
        raise ScorchAPIError(message, status=status, code=code, request_id=request_id)
    raise ScorchAPIError(
        f"Scorch API returned HTTP {status}", status=status, request_id=request_id
    )
