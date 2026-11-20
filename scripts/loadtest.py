#!/usr/bin/env python3
"""Load and abuse test for a running playground API (standard library only).

  python3 scripts/loadtest.py http://localhost:8081                 # 20 s of mixed traffic, 8 workers
  python3 scripts/loadtest.py http://localhost:8081 --duration 60 --concurrency 24
  python3 scripts/loadtest.py http://localhost:8081 --abuse         # adversarial requests, each must be refused or bounded

Start the API with the rate limits lifted (RATE_BASIC_BURST=100000 RATE_BASIC_PER_SEC=100000 and the
VERIFIED equivalents), otherwise the limiter, not the service, is what you measure.
Exit status is non-zero if a threshold fails: any 5xx, too many errors, a slow p95, or an abuse case that
was not stopped within its time bound.
"""
import argparse
import gzip
import http.client
import json
import random
import statistics
import sys
import threading
import time
import urllib.parse
import urllib.request

PEOPLE = "db = { people: { documents: [" + ",".join(
    '{name: "p%d", age: %d, city: "c%d", tags: ["a", "b"]}' % (i, 20 + i % 60, i % 7) for i in range(200)
) + "], indexes: [{key: {age: 1}}, {key: {city: 1}}] } }"

# (weight, name, path, body): the mix a busy day might look like.
MIX = [
    (30, "find", "/api/v1/run", {"script": "db.people.find({age: {$gt: 50}}, {_id: 0, name: 1}).sort({age: -1}).limit(20)", "config": PEOPLE}),
    (15, "aggregate", "/api/v1/run", {"script": "db.people.aggregate([{$group: {_id: '$city', n: {$sum: 1}, avg: {$avg: '$age'}}}, {$sort: {n: -1}}])", "config": PEOPLE}),
    (15, "crud-session", "/api/v1/run", {"script": "db.people.insertMany([{name: 'x', age: 1}, {name: 'y', age: 2}])\ndb.people.updateMany({age: {$lt: 3}}, {$set: {young: true}})\ndb.people.find({young: true})\ndb.people.deleteMany({young: true})\ndb.people.countDocuments({})", "config": PEOPLE}),
    (10, "explain", "/api/v1/explain", {"script": "db.people.find({age: 36}); db.people.find({city: 'c3'}).sort({age: 1})", "config": PEOPLE}),
    (10, "index+regex", "/api/v1/run", {"script": "db.people.createIndex({name: 1})\ndb.people.find({name: /^p1/}).count()", "config": PEOPLE}),
    (10, "validate", "/api/v1/validate", {"script": "db.people.find({age: 1})", "config": PEOPLE}),
    (5, "export", "/api/v1/export", {"script": "db.people.find()", "config": PEOPLE, "format": "mongosh"}),
    (5, "bad-operator", "/api/v1/run", {"script": "db.people.find({age: {$nope: 1}})", "config": PEOPLE}),
]


class Client:
    """One connection per worker thread; reconnects if the server closes it."""

    def __init__(self, base):
        u = urllib.parse.urlparse(base)
        self.host, self.port, self.tls = u.hostname, u.port or (443 if u.scheme == "https" else 80), u.scheme == "https"
        self.conn = None

    def post(self, path, body, headers=None, compress=False, timeout=60):
        data = json.dumps(body).encode()
        h = {"content-type": "application/json", "accept-encoding": "gzip", **(headers or {})}
        if compress:
            data = gzip.compress(data)
            h["content-encoding"] = "gzip"
        for attempt in (0, 1):
            try:
                if self.conn is None:
                    cls = http.client.HTTPSConnection if self.tls else http.client.HTTPConnection
                    self.conn = cls(self.host, self.port, timeout=timeout)
                self.conn.request("POST", path, data, h)
                res = self.conn.getresponse()
                raw = res.read()
                if res.getheader("content-encoding") == "gzip":
                    raw = gzip.decompress(raw)
                try:
                    return res.status, json.loads(raw), dict(res.getheaders())
                except ValueError:
                    return res.status, raw.decode(errors="replace")[:200], dict(res.getheaders())
            except (http.client.HTTPException, OSError):
                self.conn = None
                if attempt:
                    raise
        raise RuntimeError("unreachable")


def pct(sorted_ms, p):
    return sorted_ms[min(len(sorted_ms) - 1, int(len(sorted_ms) * p))] if sorted_ms else 0.0


def load(args):
    stop = time.time() + args.duration
    lock = threading.Lock()
    lat, status, kinds, bad = [], {}, {}, []
    weights = [m[0] for m in MIX]

    def worker():
        c = Client(args.base)
        while time.time() < stop:
            _, name, path, body = random.choices(MIX, weights)[0]
            t0 = time.perf_counter()
            try:
                st, data, _ = c.post(path, body)
                ms = (time.perf_counter() - t0) * 1000
                # a script reporting ok:false is a normal answer for "bad-operator"; anything else is a failure
                app_ok = not (isinstance(data, dict) and data.get("ok") is False) or name == "bad-operator"
                good = st < 500 and st != 429 and app_ok
            except Exception as e:  # connection errors count as failures
                ms, st, good, data = (time.perf_counter() - t0) * 1000, "conn", False, str(e)
            with lock:
                lat.append(ms)
                status[st] = status.get(st, 0) + 1
                k = kinds.setdefault(name, [0, 0])
                k[0] += 1
                k[1] += 0 if good else 1
                if not good and len(bad) < 5:
                    bad.append((name, st, str(data.get("error") if isinstance(data, dict) and data.get("error") else data)[:300]))

    t0 = time.time()
    threads = [threading.Thread(target=worker) for _ in range(args.concurrency)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    elapsed = time.time() - t0

    lat.sort()
    n = len(lat)
    fails = sum(v[1] for v in kinds.values())
    five = sum(c for s, c in status.items() if isinstance(s, int) and s >= 500) + status.get("conn", 0)
    print(f"\n{n} requests in {elapsed:.1f}s with {args.concurrency} workers = {n / elapsed:.1f} req/s")
    print(f"latency ms: p50 {pct(lat, .5):.0f}  p95 {pct(lat, .95):.0f}  p99 {pct(lat, .99):.0f}  max {lat[-1] if lat else 0:.0f}  mean {statistics.mean(lat) if lat else 0:.0f}")
    print("status codes:", dict(sorted(status.items(), key=lambda kv: str(kv[0]))))
    for name, (count, f) in sorted(kinds.items()):
        print(f"  {name:14} {count:6} requests, {f} failed")
    for b in bad:
        print("  sample failure:", b)

    problems = []
    if five:
        problems.append(f"{five} responses were 5xx or connection errors")
    if n and fails / n > 0.01:
        problems.append(f"{fails / n:.1%} of requests failed (limit 1%)")
    if pct(lat, .95) > args.p95_ms:
        problems.append(f"p95 {pct(lat, .95):.0f} ms is above {args.p95_ms} ms")
    if status.get(429):
        problems.append(f"{status[429]} requests were rate limited: start the API with the rate limits lifted")
    return problems


# ----- abuse ------------------------------------------------------------------------------------------------

def abuse(args):
    c = Client(args.base)
    problems = []

    def case(name, path, body, expect, max_s=16, compress=False):
        t0 = time.perf_counter()
        try:
            st, data, _ = c.post(path, body, compress=compress, timeout=max_s + 30)
        except Exception as e:
            st, data = "conn", str(e)
        took = time.perf_counter() - t0
        ok, why = expect(st, data)
        if took > max_s:
            ok, why = False, f"took {took:.1f}s, over the {max_s}s bound"
        print(f"{'PASS' if ok else 'FAIL'}  {name}  ({took:.1f}s, HTTP {st})" + ("" if ok else f"   -> {why}: {str(data)[:200]}"))
        if not ok:
            problems.append(name)

    refused = lambda codes: (lambda st, d: (st in codes, f"expected HTTP {codes}"))
    small = "db = { c: [{n: 1}] }"
    run = "/api/v1/run"

    case("payload over 32 MB (sent gzipped)", run, {"script": "db.c.find()", "config": {"c": [{"s": "x" * (33 * 1024 * 1024)}]}}, refused({413, 422}), compress=True)
    case("nesting 300 levels deep", run, {"script": "db.c.find(" + "{a:" * 300 + "1" + "}" * 300 + ")", "config": small}, refused({422}))
    case("26 statements", run, {"script": "\n".join(["db.c.find()"] * 26), "config": small}, refused({422}))
    case("9 collections", run, {"script": "\n".join(f"db.c{i}.find()" for i in range(9)), "config": small}, refused({422}))
    case("257 documents", run, {"script": "db.c.find()", "config": {"c": [{"i": i} for i in range(257)]}}, refused({422}))
    case("4 indexes", run, {"script": "db.c.createIndex({a:1}); db.c.createIndex({b:1}); db.c.createIndex({c:1}); db.c.createIndex({d:1})", "config": small}, refused({422}))
    case("$where", run, {"script": "db.c.find({$where: 'sleep(5000)'})", "config": small}, refused({403}))
    case("$function", run, {"script": "db.c.aggregate([{$addFields: {x: {$function: {body: 'function(){return 1}', args: [], lang: 'js'}}}}])", "config": small}, refused({403}))
    case("$out", run, {"script": "db.c.aggregate([{$out: 'stolen'}])", "config": small}, refused({403}))
    case("$merge", run, {"script": "db.c.aggregate([{$merge: {into: 'stolen'}}])", "config": small}, refused({403}))
    case("cross-database $lookup", run, {"script": "db.c.aggregate([{$lookup: {from: {db: 'admin', coll: 'system.users'}, pipeline: [], as: 'x'}}])", "config": small}, refused({403}))
    case("admin stage $currentOp", run, {"script": "db.c.aggregate([{$currentOp: {}}])", "config": small}, refused({403}))
    case("regex literal longer than 256 chars", run, {"script": "db.c.find({s: /" + "a" * 300 + "/})", "config": small}, refused({422}))
    case("malformed JSON body", run, "not an object", refused({422}))
    case("unknown method", run, {"script": "db.c.dropDatabase()", "config": small}, refused({422}))
    case("catastrophic regex is bounded by the time limit", run,
         {"script": "db.c.find({s: /(a+)+$/})", "config": {"c": [{"s": "a" * 40 + "b"}] * 200}}, lambda st, d: (st in (200, 408, 422), "expected a bounded answer"))
    case("CPU-bound aggregation stops at maxTimeMS", run,
         {"script": "db.c.aggregate([{$addFields: {x: {$range: [0, 2000000]}}}, {$unwind: '$x'}, {$group: {_id: null, n: {$sum: 1}}}])", "config": {"c": [{"i": i} for i in range(256)]}},
         lambda st, d: (st == 200 and isinstance(d, dict) and (d.get("ok") is False or d.get("ok") is True), "expected a bounded answer"), max_s=20)
    case("result explosion hits the 64 MB ceiling", run,
         {"script": "db.c.aggregate([{$addFields: {x: {$range: [0, 60000]}}}, {$unwind: '$x'}])", "config": {"c": [{"i": i} for i in range(256)]}},
         lambda st, d: (st == 200 and isinstance(d, dict) and d.get("ok") is False, "expected the script to stop with an error"), max_s=30)
    case("search index run without Atlas Search is refused clearly", run,
         {"script": "db.c.find()", "config": {"c": {"documents": [{"a": 1}], "searchIndexes": [{"definition": {"mappings": {"dynamic": True}}}]}}},
         lambda st, d: (st in (200, 422, 503), "expected a clear answer"), max_s=70)

    # the service must still be healthy after all of that
    st = urllib.request.urlopen(args.base.rstrip("/") + "/readyz", timeout=10).status
    print(f"{'PASS' if st == 200 else 'FAIL'}  /readyz after the abuse")
    if st != 200:
        problems.append("readyz")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base", nargs="?", default="http://localhost:8081")
    ap.add_argument("--duration", type=int, default=20)
    ap.add_argument("--concurrency", type=int, default=8)
    ap.add_argument("--p95-ms", type=int, default=2000)
    ap.add_argument("--abuse", action="store_true", help="run the adversarial cases instead of the load mix")
    args = ap.parse_args()
    problems = abuse(args) if args.abuse else load(args)
    if problems:
        print("\nFAILED:", "; ".join(problems))
        sys.exit(1)
    print("\nOK")


if __name__ == "__main__":
    main()
