#!/usr/bin/env python3
"""Live terminal dashboard for concurrent Scorch search requests."""

import argparse
import json
import math
import os
import shutil
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

DEFAULT_QUERIES = [
    "Rust programming language",
    "Tokio asynchronous runtime",
    "Axum web framework",
    "Chromium DevTools Protocol",
    "Web scraping best practices",
    "Model Context Protocol",
    "Nix reproducible development",
    "Jujutsu version control",
]
SPINNER = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"


@dataclass
class SearchTask:
    index: int
    query: str
    request_id: str
    status: str = "queued"
    started_at: float | None = None
    finished_at: float | None = None
    provider: str = "-"
    result_count: int = 0
    error: str = ""
    lock: threading.Lock = field(default_factory=threading.Lock, repr=False)

    def snapshot(self, now):
        with self.lock:
            elapsed = 0.0
            if self.started_at is not None:
                elapsed = (self.finished_at or now) - self.started_at
            return {
                "index": self.index,
                "query": self.query,
                "request_id": self.request_id,
                "status": self.status,
                "elapsed": elapsed,
                "provider": self.provider,
                "result_count": self.result_count,
                "error": self.error,
            }


def parse_args():
    parser = argparse.ArgumentParser(
        description="Visually demonstrate concurrent requests to Scorch /v1/search."
    )
    parser.add_argument("queries", nargs="*", help="queries to cycle through")
    parser.add_argument(
        "--api-url",
        default=os.environ.get("SCORCH_API_URL", "http://127.0.0.1:3000"),
    )
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument(
        "--requests",
        type=int,
        help="total requests; defaults to 8 or the number of supplied queries",
    )
    parser.add_argument("--limit", type=int, default=3, help="results per search")
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument(
        "--scrape",
        action="store_true",
        help="also scrape each result (uses substantially more resources)",
    )
    parser.add_argument("--plain", action="store_true", help="disable the live dashboard")
    parser.add_argument("--no-color", action="store_true")
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("--concurrency must be at least 1")
    if args.requests is not None and args.requests < 1:
        parser.error("--requests must be at least 1")
    if not 1 <= args.limit <= 20:
        parser.error("--limit must be between 1 and 20")
    if args.timeout <= 0:
        parser.error("--timeout must be greater than zero")
    return args


def color(text, code, enabled):
    return f"\033[{code}m{text}\033[0m" if enabled else text


def truncate(text, width):
    if len(text) <= width:
        return text.ljust(width)
    return f"{text[: max(0, width - 1)]}…"


def check_health(api_url, timeout):
    request = urllib.request.Request(f"{api_url.rstrip('/')}/healthz")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        payload = json.load(response)
    if payload.get("status") != "ok":
        raise RuntimeError(f"unexpected health response: {payload}")


def execute_search(task, endpoint, limit, scrape, timeout):
    with task.lock:
        task.status = "running"
        task.started_at = time.monotonic()

    payload = {
        "query": task.query,
        "limit": limit,
        "country": "us",
        "language": "en",
    }
    if scrape:
        payload["scrapeOptions"] = {}
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload).encode(),
        headers={
            "content-type": "application/json",
            "x-request-id": task.request_id,
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            result = json.load(response)
        with task.lock:
            task.provider = result.get("provider", "-")
            task.result_count = len(result.get("results", []))
            task.status = "done"
    except urllib.error.HTTPError as error:
        try:
            payload = json.loads(error.read().decode())
            message = payload.get("message", str(error))
        except (UnicodeDecodeError, json.JSONDecodeError):
            message = str(error)
        with task.lock:
            task.error = f"HTTP {error.code}: {message}"
            task.status = "error"
    except Exception as error:  # Network failures vary by platform.
        with task.lock:
            task.error = str(error)
            task.status = "error"
    finally:
        with task.lock:
            task.finished_at = time.monotonic()


def render_dashboard(snapshots, started_at, concurrency, peak_active, colors):
    now = time.monotonic()
    counts = {
        status: sum(item["status"] == status for item in snapshots)
        for status in ("queued", "running", "done", "error")
    }
    finished = counts["done"] + counts["error"]
    width = shutil.get_terminal_size((100, 24)).columns
    bar_width = max(12, min(36, width - 58))
    filled = round(bar_width * finished / len(snapshots))
    progress = "█" * filled + "░" * (bar_width - filled)

    print("\033[2J\033[H", end="")
    print(color("Scorch concurrent search dashboard", "1;36", colors))
    print(
        f"Elapsed {now - started_at:6.1f}s  "
        f"Concurrency {concurrency}  Peak active {peak_active}  "
        f"[{progress}] {finished}/{len(snapshots)}"
    )
    print(
        f"Queued {counts['queued']}  Running {counts['running']}  "
        f"Done {counts['done']}  Errors {counts['error']}"
    )
    print("─" * min(width, 110))

    spinner = SPINNER[int(now * 10) % len(SPINNER)]
    query_width = max(20, min(42, width - 64))
    for item in snapshots:
        if item["status"] == "queued":
            icon = color("○ QUEUED ", "2", colors)
            detail = "waiting for worker"
        elif item["status"] == "running":
            icon = color(f"{spinner} RUNNING", "36", colors)
            detail = f"{item['elapsed']:6.2f}s"
        elif item["status"] == "done":
            icon = color("✓ DONE   ", "32", colors)
            detail = (
                f"{item['elapsed']:6.2f}s  provider={item['provider']:<10} "
                f"results={item['result_count']}"
            )
        else:
            icon = color("✗ ERROR  ", "31", colors)
            detail = f"{item['elapsed']:6.2f}s  {item['error']}"
        print(
            f"{icon}  {item['request_id']:<20}  "
            f"{truncate(item['query'], query_width)}  {detail}"
        )
    sys.stdout.flush()


def print_plain_changes(snapshots, previous, started_at):
    for item in snapshots:
        index = item["index"]
        if previous.get(index) == item["status"]:
            continue
        previous[index] = item["status"]
        detail = ""
        if item["status"] == "done":
            detail = f" provider={item['provider']} results={item['result_count']}"
        elif item["status"] == "error":
            detail = f" error={item['error']}"
        print(
            f"[{time.monotonic() - started_at:6.2f}s] {item['request_id']} "
            f"{item['status'].upper():7} {item['query']}{detail}",
            flush=True,
        )


def print_summary(snapshots, wall_time, peak_active, colors):
    completed = [item for item in snapshots if item["status"] == "done"]
    failed = [item for item in snapshots if item["status"] == "error"]
    durations = sorted(item["elapsed"] for item in completed)
    print()
    print(color("Summary", "1;36", colors))
    print(f"  Wall time:    {wall_time:.2f}s")
    print(f"  Peak active:  {peak_active}")
    print(f"  Successful:   {len(completed)}")
    print(f"  Failed:       {len(failed)}")
    if durations:
        p95_index = max(0, math.ceil(len(durations) * 0.95) - 1)
        print(f"  Median:       {statistics.median(durations):.2f}s")
        print(f"  p95:          {durations[p95_index]:.2f}s")
        print(f"  Throughput:   {len(completed) / wall_time:.2f} requests/s")


def main():
    args = parse_args()
    queries = args.queries or DEFAULT_QUERIES
    request_count = args.requests or (len(args.queries) if args.queries else 8)
    tasks = [
        SearchTask(
            index=index,
            query=queries[index % len(queries)],
            request_id=f"search-demo-{index + 1:02d}",
        )
        for index in range(request_count)
    ]
    endpoint = f"{args.api_url.rstrip('/')}/v1/search"
    dashboard = sys.stdout.isatty() and not args.plain
    colors = dashboard and not args.no_color and "NO_COLOR" not in os.environ

    try:
        check_health(args.api_url, min(args.timeout, 5.0))
    except Exception as error:
        print(f"Scorch is not reachable at {args.api_url}: {error}", file=sys.stderr)
        return 2

    started_at = time.monotonic()
    previous = {}
    peak_active = 0
    with ThreadPoolExecutor(max_workers=args.concurrency) as executor:
        futures = [
            executor.submit(
                execute_search,
                task,
                endpoint,
                args.limit,
                args.scrape,
                args.timeout,
            )
            for task in tasks
        ]
        while not all(future.done() for future in futures):
            now = time.monotonic()
            snapshots = [task.snapshot(now) for task in tasks]
            peak_active = max(
                peak_active,
                sum(item["status"] == "running" for item in snapshots),
            )
            if dashboard:
                render_dashboard(
                    snapshots, started_at, args.concurrency, peak_active, colors
                )
            else:
                print_plain_changes(snapshots, previous, started_at)
            time.sleep(0.1)

    finished_at = time.monotonic()
    snapshots = [task.snapshot(finished_at) for task in tasks]
    peak_active = max(
        peak_active, sum(item["status"] == "running" for item in snapshots)
    )
    if dashboard:
        render_dashboard(snapshots, started_at, args.concurrency, peak_active, colors)
    else:
        print_plain_changes(snapshots, previous, started_at)
    print_summary(snapshots, finished_at - started_at, peak_active, colors)
    return 1 if any(item["status"] == "error" for item in snapshots) else 0


if __name__ == "__main__":
    raise SystemExit(main())
