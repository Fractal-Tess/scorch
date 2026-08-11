import json
import math
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from scorch_client import (
    ScorchAPIError,
    ScorchClient,
    ScorchConnectionError,
    ScorchResponseError,
)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def _body(self):
        length = int(self.headers.get("content-length", "0"))
        return json.loads(self.rfile.read(length)) if length else None

    def _json(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.send_header("x-request-id", "test-request")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if self.path == "/health" and self.headers.get("x-slow-headers-test"):
            payload = b'{"status":"ok"}'
            pieces = [
                b"HTTP/1.0 200 OK\r\n",
                b"Content-Type: application/json\r\n",
                f"Content-Length: {len(payload)}\r\n".encode(),
                b"\r\n",
                payload,
            ]
            try:
                for piece in pieces:
                    self.wfile.write(piece)
                    self.wfile.flush()
                    time.sleep(0.1)
            except (BrokenPipeError, ConnectionResetError):
                pass
        elif self.path == "/health" and self.headers.get("x-redirect-test"):
            self.send_response(302)
            self.send_header("location", "http://127.0.0.1:9/capture")
            self.send_header("content-length", "0")
            self.end_headers()
        elif self.path == "/health" and self.headers.get("x-truncated-test"):
            payload = b'{"status":"ok"}'
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload) + 10))
            self.end_headers()
            self.wfile.write(payload)
            self.close_connection = True
        elif self.path == "/health" and self.headers.get("x-trickle-test"):
            payload = b'{"status":"ok"}'
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            try:
                for byte in payload:
                    self.wfile.write(bytes([byte]))
                    self.wfile.flush()
                    time.sleep(0.03)
            except (BrokenPipeError, ConnectionResetError):
                pass
        elif self.path == "/health":
            self._json(200, {"status": "ok"})
        elif self.path == "/ready":
            self._json(
                200,
                {
                    "status": "ready",
                    "browserAvailable": True,
                    "browser": "obscura",
                    "obscuraStealth": True,
                    "maxConcurrency": 4,
                    "searchProvider": "metasearch",
                    "searchEngines": ["bing", "duckduckgo"],
                },
            )
        elif self.path.startswith("/v1/crawls/job-id?"):
            self._json(
                200,
                {
                    "id": "job-id",
                    "status": "running",
                    "createdAtMs": 1,
                    "expiresAtMs": 2,
                    "total": 1,
                    "completed": 0,
                    "errorCount": 0,
                    "cursor": 0,
                    "documents": [],
                    "errors": [],
                },
            )
        else:
            self._json(404, {"code": "not_found", "message": "missing"})

    def do_POST(self):
        body = self._body()
        if self.path == "/v1/search":
            if body["query"] == "bad":
                self._json(
                    400,
                    {"code": "invalid_request", "message": "bad query"},
                )
                return
            self._json(
                200,
                {
                    "query": body["query"],
                    "provider": "metasearch",
                    "engines": body.get("engines", ["bing", "duckduckgo"]),
                    "results": [],
                    "elapsedMs": 1,
                },
            )
        elif self.path == "/v1/scrape":
            self._json(
                200,
                {
                    "url": body["url"],
                    "finalUrl": body["url"],
                    "engine": "fetch",
                    "elapsedMs": 1,
                    "metadata": {"statusCode": 200},
                    "markdown": "ok",
                },
            )
        elif self.path == "/v1/map":
            self._json(
                200,
                {"url": body["url"], "links": [], "elapsedMs": 1, "sources": []},
            )
        elif self.path == "/v1/crawls":
            self._json(
                202,
                {
                    "id": "job-id",
                    "status": "queued",
                    "createdAtMs": 1,
                    "expiresAtMs": 2,
                    "total": 0,
                    "completed": 0,
                    "errorCount": 0,
                },
            )
        else:
            self._json(404, {"code": "not_found", "message": "missing"})

    def do_DELETE(self):
        if self.path == "/v1/crawls/job-id":
            self._json(200, {"id": "job-id", "deleted": True})
        else:
            self._json(404, {"code": "not_found", "message": "missing"})


class ClientTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        host, port = cls.server.server_address
        cls.base_url = f"http://{host}:{port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def test_health_readiness_and_search(self):
        client = ScorchClient(self.base_url)
        self.assertEqual(client.health()["status"], "ok")
        self.assertEqual(client.readiness()["browser"], "obscura")
        response = client.search(
            "Rust", country="bg", language="en", engines=["wikipedia"]
        )
        self.assertEqual(response["engines"], ["wikipedia"])

    def test_scrape_map_and_crawl(self):
        client = ScorchClient(self.base_url)
        self.assertEqual(client.scrape("https://example.com")["markdown"], "ok")
        self.assertEqual(client.map("https://example.com")["links"], [])
        job = client.start_crawl("https://example.com")
        self.assertEqual(client.crawl_status(job["id"])["status"], "running")
        self.assertTrue(client.cancel_crawl(job["id"])["deleted"])

    def test_structured_api_error(self):
        client = ScorchClient(self.base_url)
        with self.assertRaises(ScorchAPIError) as caught:
            client.search("bad")
        self.assertEqual(caught.exception.status, 400)
        self.assertEqual(caught.exception.code, "invalid_request")
        self.assertEqual(caught.exception.request_id, "test-request")

    def test_response_limit_redirect_deadline_and_url_validation(self):
        with self.assertRaises(ScorchResponseError):
            ScorchClient(self.base_url, max_response_bytes=5).health()
        with self.assertRaises(ScorchResponseError):
            ScorchClient(self.base_url, headers={"x-truncated-test": "true"}).health()
        with self.assertRaises(ScorchAPIError) as redirected:
            ScorchClient(
                self.base_url,
                headers={
                    "Authorization": "Bearer secret",
                    "x-redirect-test": "true",
                },
            ).health()
        self.assertEqual(redirected.exception.status, 302)
        with self.assertRaises(ScorchConnectionError):
            ScorchClient(
                self.base_url,
                timeout=0.08,
                headers={"x-trickle-test": "true"},
            ).health()
        started = time.monotonic()
        with self.assertRaises(ScorchConnectionError):
            ScorchClient(
                self.base_url,
                timeout=0.08,
                headers={"x-slow-headers-test": "true"},
            ).health()
        self.assertLess(time.monotonic() - started, 0.25)
        with self.assertRaises(ValueError):
            ScorchClient("https://user:secret@example.com")
        with self.assertRaises(ValueError):
            ScorchClient("file:///tmp/scorch.sock")
        with self.assertRaises(ValueError):
            ScorchClient("")
        with self.assertRaises(ValueError):
            ScorchClient("https://example.com?")
        with self.assertRaises(ValueError):
            ScorchClient(self.base_url, timeout=math.nan)
        with self.assertRaises(ValueError):
            ScorchClient(self.base_url, timeout=math.inf)
        with self.assertRaises(ValueError):
            ScorchClient(self.base_url, max_response_bytes=1.5)  # type: ignore[arg-type]
        with self.assertRaises(ValueError):
            ScorchClient(self.base_url, max_response_bytes=math.nan)  # type: ignore[arg-type]
        with self.assertRaises(ValueError):
            ScorchClient(self.base_url, max_response_bytes=math.inf)  # type: ignore[arg-type]


if __name__ == "__main__":
    unittest.main()
