"""Small async load generator for the stress pass.

Two modes:

  load   N concurrent workers hammer one URL for D seconds.
  probe  One request every INTERVAL seconds to a cheap URL, recording latency.

Run them as separate processes. A probe sharing the load generator's event loop or
connection pool would queue behind load traffic on the *client*, and a stall would be
indistinguishable from the client being busy. Separate processes, separate sockets: any
stall the probe sees happened in the server.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import statistics
import sys
import time
import uuid
from collections import Counter

import httpx


def pct(values: list[float], p: float) -> float:
    if not values:
        return float("nan")
    ordered = sorted(values)
    k = min(len(ordered) - 1, max(0, round(p / 100 * (len(ordered) - 1))))
    return ordered[k]


def summarize(label: str, latencies: list[float], statuses: Counter, elapsed: float) -> dict:
    ms = [x * 1000 for x in latencies]
    return {
        "label": label,
        "requests": sum(statuses.values()),
        "rps": round(sum(statuses.values()) / elapsed, 1) if elapsed else 0,
        "p50_ms": round(pct(ms, 50), 1),
        "p95_ms": round(pct(ms, 95), 1),
        "p99_ms": round(pct(ms, 99), 1),
        "max_ms": round(max(ms), 1) if ms else None,
        "mean_ms": round(statistics.fmean(ms), 1) if ms else None,
        "statuses": dict(statuses),
    }


async def run_load(args) -> dict:
    latencies: list[float] = []
    statuses: Counter = Counter()
    deadline = time.monotonic() + args.duration
    limits = httpx.Limits(max_connections=args.concurrency + 8,
                          max_keepalive_connections=args.concurrency + 8)
    timeout = httpx.Timeout(args.timeout)
    counter = 0

    async with httpx.AsyncClient(limits=limits, timeout=timeout) as client:

        async def worker():
            nonlocal counter
            while time.monotonic() < deadline:
                counter += 1
                body = None
                if args.body:
                    body = args.body.replace("{uniq}", uuid.uuid4().hex)
                started = time.monotonic()
                try:
                    if args.method == "GET":
                        r = await client.get(args.url)
                    else:
                        r = await client.request(
                            args.method, args.url, content=body,
                            headers={"content-type": "application/json"},
                        )
                    statuses[r.status_code] += 1
                except httpx.TimeoutException:
                    statuses["timeout"] += 1
                except httpx.HTTPError as exc:
                    statuses[type(exc).__name__] += 1
                latencies.append(time.monotonic() - started)

        started = time.monotonic()
        await asyncio.gather(*(worker() for _ in range(args.concurrency)))
        elapsed = time.monotonic() - started

    return summarize(f"load c={args.concurrency}", latencies, statuses, elapsed)


async def run_probe(args) -> dict:
    latencies: list[float] = []
    statuses: Counter = Counter()
    deadline = time.monotonic() + args.duration
    started_all = time.monotonic()
    # Fresh connection each time is closer to "a new visitor arrives" than one warm socket.
    async with httpx.AsyncClient(timeout=httpx.Timeout(args.timeout)) as client:
        while time.monotonic() < deadline:
            started = time.monotonic()
            try:
                r = await client.get(args.url)
                statuses[r.status_code] += 1
            except httpx.TimeoutException:
                statuses["timeout"] += 1
            except httpx.HTTPError as exc:
                statuses[type(exc).__name__] += 1
            latencies.append(time.monotonic() - started)
            await asyncio.sleep(max(0.0, args.interval - (time.monotonic() - started)))
    return summarize("probe", latencies, statuses, time.monotonic() - started_all)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("load", "probe"))
    parser.add_argument("url")
    parser.add_argument("--concurrency", type=int, default=10)
    parser.add_argument("--duration", type=float, default=10.0)
    parser.add_argument("--interval", type=float, default=0.1)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--method", default="GET")
    parser.add_argument("--body", default=None, help="'{uniq}' is replaced per request")
    args = parser.parse_args()

    result = asyncio.run(run_load(args) if args.mode == "load" else run_probe(args))
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
