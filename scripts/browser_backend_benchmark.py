#!/usr/bin/env python3
"""Compare Scorch browser backends with sequential and parallel scrape jobs.

The benchmark starts a fresh scorchd process for each backend, forces browser
rendering, and samples the complete daemon process tree from /proc. Reported
PSS is the best estimate of physical memory; summed RSS is included for easier
comparison with common process-monitoring tools.
"""

import argparse
import json
import math
import os
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

DEFAULT_URLS = [
    "https://example.com/",
    "https://quotes.toscrape.com/js/",
    "https://www.rust-lang.org/",
    "https://news.ycombinator.com/",
]
PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")


@dataclass
class RequestResult:
    url: str
    wall_seconds: float
    server_ms: int | None
    engine: str | None
    response_bytes: int
    error: str | None


class ProcessTreeMonitor:
    def __init__(self, root_pid, interval=0.05):
        self.root_pid = root_pid
        self.interval = interval
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.peak_rss = 0
        self.peak_pss = 0
        self.peak_processes = 0
        self.latest = {"rss": 0, "pss": 0, "processes": 0}
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self.thread.start()

    def reset_phase(self):
        sample = sample_process_tree(self.root_pid)
        with self.lock:
            self.peak_rss = sample[0]
            self.peak_pss = sample[1]
            self.peak_processes = sample[2]
            self.latest = dict(zip(("rss", "pss", "processes"), sample))

    def snapshot(self):
        with self.lock:
            return {
                "peak_rss_bytes": self.peak_rss,
                "peak_pss_bytes": self.peak_pss,
                "peak_processes": self.peak_processes,
                "final_rss_bytes": self.latest["rss"],
                "final_pss_bytes": self.latest["pss"],
                "final_processes": self.latest["processes"],
            }

    def stop(self):
        self.stop_event.set()
        self.thread.join(timeout=2)

    def _run(self):
        while not self.stop_event.is_set():
            sample = sample_process_tree(self.root_pid)
            with self.lock:
                self.latest = dict(zip(("rss", "pss", "processes"), sample))
                self.peak_rss = max(self.peak_rss, sample[0])
                self.peak_pss = max(self.peak_pss, sample[1])
                self.peak_processes = max(self.peak_processes, sample[2])
            self.stop_event.wait(self.interval)


def process_table():
    table = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text()
            tail = stat[stat.rfind(")") + 2 :].split()
            table[int(entry.name)] = int(tail[1])
        except (FileNotFoundError, PermissionError, IndexError, ValueError):
            continue
    return table


def process_tree_pids(root_pid):
    table = process_table()
    children = {}
    for pid, parent in table.items():
        children.setdefault(parent, []).append(pid)
    found = []
    pending = [root_pid]
    while pending:
        pid = pending.pop()
        if pid in found or pid not in table:
            continue
        found.append(pid)
        pending.extend(children.get(pid, ()))
    return found


def read_rss(pid):
    try:
        fields = Path(f"/proc/{pid}/statm").read_text().split()
        return int(fields[1]) * PAGE_SIZE
    except (FileNotFoundError, PermissionError, IndexError, ValueError):
        return 0


def read_pss(pid):
    try:
        with Path(f"/proc/{pid}/smaps_rollup").open() as handle:
            for line in handle:
                if line.startswith("Pss:"):
                    return int(line.split()[1]) * 1024
    except (FileNotFoundError, PermissionError, ProcessLookupError, ValueError):
        pass
    return 0


def sample_process_tree(root_pid):
    pids = process_tree_pids(root_pid)
    return (
        sum(read_rss(pid) for pid in pids),
        sum(read_pss(pid) for pid in pids),
        len(pids),
    )


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def request_json(url, payload=None, timeout=30):
    body = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(
        url,
        data=body,
        headers={"content-type": "application/json"} if body else {},
        method="POST" if body else "GET",
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)


def wait_until_ready(api_url, process, timeout):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"scorchd exited with status {process.returncode}")
        try:
            response = request_json(f"{api_url}/readyz", timeout=1)
            if response.get("status") == "ready":
                return
        except Exception as error:  # Startup connection failures vary by platform.
            last_error = error
        time.sleep(0.05)
    raise RuntimeError(f"scorchd did not become ready: {last_error}")


def scrape(api_url, backend, url, timeout):
    started = time.monotonic()
    try:
        response = request_json(
            f"{api_url}/v1/scrape",
            {
                "url": url,
                "options": {
                    "formats": ["html"],
                    "render": "always",
                    "browser": backend,
                    "timeoutMs": round(timeout * 1000),
                    "onlyMainContent": False,
                    "blockMedia": True,
                },
            },
            timeout=timeout + 5,
        )
        encoded = json.dumps(response, separators=(",", ":")).encode()
        engine = response.get("engine")
        error = None if engine == backend else f"expected {backend}, got {engine}"
        return RequestResult(
            url=url,
            wall_seconds=time.monotonic() - started,
            server_ms=response.get("elapsedMs"),
            engine=engine,
            response_bytes=len(encoded),
            error=error,
        )
    except urllib.error.HTTPError as error:
        try:
            detail = json.load(error).get("message", str(error))
        except Exception:
            detail = str(error)
        message = f"HTTP {error.code}: {detail}"
    except Exception as error:  # Network failures vary by platform.
        message = str(error)
    return RequestResult(
        url=url,
        wall_seconds=time.monotonic() - started,
        server_ms=None,
        engine=None,
        response_bytes=0,
        error=message,
    )


def run_workload(api_url, backend, urls, concurrency, rounds, timeout):
    jobs = urls * rounds
    started = time.monotonic()
    if concurrency == 1:
        results = [scrape(api_url, backend, url, timeout) for url in jobs]
    else:
        with ThreadPoolExecutor(max_workers=concurrency) as executor:
            results = list(
                executor.map(
                    lambda url: scrape(api_url, backend, url, timeout),
                    jobs,
                )
            )
    return results, time.monotonic() - started


def percentile(values, percentile_value):
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * percentile_value) - 1)
    return ordered[index]


def summarize_phase(results, wall_seconds, memory):
    successful = [result for result in results if result.error is None]
    durations = [result.wall_seconds for result in successful]
    return {
        "requests": len(results),
        "successful": len(successful),
        "failed": len(results) - len(successful),
        "wall_seconds": wall_seconds,
        "median_seconds": statistics.median(durations) if durations else None,
        "p95_seconds": percentile(durations, 0.95),
        "throughput_per_second": len(successful) / wall_seconds if wall_seconds else 0,
        "response_bytes": sum(result.response_bytes for result in successful),
        "memory": memory,
        "errors": [
            {"url": result.url, "error": result.error}
            for result in results
            if result.error is not None
        ],
        "results": [result.__dict__ for result in results],
    }


def benchmark_backend(args, backend, port):
    api_url = f"http://127.0.0.1:{port}"
    command = [
        str(args.binary),
        "--bind",
        f"127.0.0.1:{port}",
        "--browser",
        backend,
        "--allowed-browsers",
        backend,
        "--max-concurrency",
        str(args.concurrency),
    ]
    env = os.environ.copy()
    env["RUST_LOG"] = args.rust_log
    env["RUST_BACKTRACE"] = "0"
    with tempfile.TemporaryFile(mode="w+") as logs:
        process = subprocess.Popen(command, stdout=logs, stderr=logs, env=env)
        monitor = ProcessTreeMonitor(process.pid)
        monitor.start()
        try:
            wait_until_ready(api_url, process, args.startup_timeout)
            time.sleep(0.25)
            idle_rss, idle_pss, idle_processes = sample_process_tree(process.pid)

            warmup = scrape(api_url, backend, args.urls[0], args.timeout)
            if warmup.error:
                raise RuntimeError(f"warmup failed: {warmup.error}")
            time.sleep(0.25)
            warm_rss, warm_pss, warm_processes = sample_process_tree(process.pid)

            monitor.reset_phase()
            sequential_results, sequential_wall = run_workload(
                api_url, backend, args.urls, 1, args.sequential_rounds, args.timeout
            )
            sequential_memory = monitor.snapshot()

            time.sleep(args.cooldown)
            monitor.reset_phase()
            parallel_results, parallel_wall = run_workload(
                api_url,
                backend,
                args.urls,
                args.concurrency,
                args.parallel_rounds,
                args.timeout,
            )
            parallel_memory = monitor.snapshot()

            return {
                "backend": backend,
                "server_pid": process.pid,
                "idle": {
                    "rss_bytes": idle_rss,
                    "pss_bytes": idle_pss,
                    "processes": idle_processes,
                },
                "post_warmup": {
                    "rss_bytes": warm_rss,
                    "pss_bytes": warm_pss,
                    "processes": warm_processes,
                    "wall_seconds": warmup.wall_seconds,
                },
                "sequential": summarize_phase(
                    sequential_results, sequential_wall, sequential_memory
                ),
                "parallel": summarize_phase(
                    parallel_results, parallel_wall, parallel_memory
                ),
            }
        except Exception:
            logs.seek(0)
            server_logs = logs.read()
            if server_logs:
                print(server_logs, file=sys.stderr)
            raise
        finally:
            monitor.stop()
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def mib(value):
    return value / (1024 * 1024)


def format_seconds(value):
    return "-" if value is None else f"{value:.2f} s"


def print_report(report):
    print("\nScorch browser backend benchmark")
    print(f"Host: {report['host']['cpu_count']} logical CPUs")
    print(f"URLs: {len(report['config']['urls'])}")
    print(
        f"Parallel workload: {report['config']['parallel_rounds']} round(s), "
        f"concurrency {report['config']['concurrency']}"
    )
    print()
    header = (
        f"{'Backend':<10} {'Workload':<11} {'Wall':>9} {'Median':>10} "
        f"{'p95':>10} {'Req/s':>8} {'Peak PSS':>11} {'Σ peak RSS':>12} {'Procs':>7}"
    )
    print(header)
    print("-" * len(header))
    for backend in report["backends"]:
        for phase_name in ("sequential", "parallel"):
            phase = backend[phase_name]
            memory = phase["memory"]
            print(
                f"{backend['backend']:<10} {phase_name:<11} "
                f"{phase['wall_seconds']:>8.2f}s "
                f"{format_seconds(phase['median_seconds']):>10} "
                f"{format_seconds(phase['p95_seconds']):>10} "
                f"{phase['throughput_per_second']:>8.2f} "
                f"{mib(memory['peak_pss_bytes']):>9.1f} MiB "
                f"{mib(memory['peak_rss_bytes']):>10.1f} MiB "
                f"{memory['peak_processes']:>7}"
            )
    print()
    for backend in report["backends"]:
        idle = backend["idle"]
        warm = backend["post_warmup"]
        print(
            f"{backend['backend']}: idle PSS {mib(idle['pss_bytes']):.1f} MiB; "
            f"post-warmup PSS {mib(warm['pss_bytes']):.1f} MiB; "
            f"warmup {warm['wall_seconds']:.2f}s"
        )
        for phase_name in ("sequential", "parallel"):
            phase = backend[phase_name]
            if phase["failed"]:
                print(f"  {phase_name}: {phase['failed']} failure(s)")


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path("target/release/scorchd"),
        help="scorchd binary to benchmark",
    )
    parser.add_argument("--urls", nargs="+", default=DEFAULT_URLS)
    parser.add_argument(
        "--backends",
        nargs="+",
        choices=("obscura", "chromium"),
        default=["obscura", "chromium"],
        help="backends to run, in order",
    )
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--sequential-rounds", type=int, default=1)
    parser.add_argument("--parallel-rounds", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=45)
    parser.add_argument("--startup-timeout", type=float, default=10)
    parser.add_argument("--cooldown", type=float, default=0.5)
    parser.add_argument("--rust-log", default="warn")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("--concurrency must be at least 1")
    if args.sequential_rounds < 1 or args.parallel_rounds < 1:
        parser.error("round counts must be at least 1")
    if not args.binary.is_file():
        parser.error(f"binary does not exist: {args.binary}")
    return args


def main():
    args = parse_args()
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {
            "cpu_count": os.cpu_count(),
            "platform": os.uname().sysname + " " + os.uname().release,
        },
        "config": {
            "binary": str(args.binary),
            "urls": args.urls,
            "backends": args.backends,
            "concurrency": args.concurrency,
            "sequential_rounds": args.sequential_rounds,
            "parallel_rounds": args.parallel_rounds,
            "timeout_seconds": args.timeout,
            "memory_method": "summed Linux /proc PSS and RSS for the scorchd process tree",
        },
        "backends": [],
    }
    for backend in args.backends:
        print(f"Benchmarking {backend}...", flush=True)
        report["backends"].append(benchmark_backend(args, backend, free_port()))
    print_report(report)
    if args.output:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"\nRaw results: {args.output}")
    return 1 if any(
        phase["failed"]
        for backend in report["backends"]
        for phase in (backend["sequential"], backend["parallel"])
    ) else 0


if __name__ == "__main__":
    raise SystemExit(main())
