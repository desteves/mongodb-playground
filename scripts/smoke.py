#!/usr/bin/env python3
"""HTTP smoke test for a running playground API.

Usage: python3 scripts/smoke.py [base-url]    (default http://localhost:8080)

Exercises the public surface the way an agent would: discovery, validation, every mode,
explain, compressed requests and responses, and the MCP endpoint. Exits non-zero on failure.
"""
import gzip
import json
import sys
import urllib.error
import urllib.request

BASE = (sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080").rstrip("/")
results = []


def call(method, path, body=None, headers=None, compress=False):
    """Return (status, headers, parsed body). Never raises on HTTP errors."""
    h = dict(headers or {})
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        h.setdefault("Content-Type", "application/json")
        if compress:
            data = gzip.compress(data)
            h["Content-Encoding"] = "gzip"
    req = urllib.request.Request(BASE + path, data=data, method=method, headers=h)
    try:
        resp = urllib.request.urlopen(req, timeout=60)
    except urllib.error.HTTPError as e:
        resp = e
    raw = resp.read()
    if resp.headers.get("Content-Encoding") == "gzip":
        raw = gzip.decompress(raw)
    try:
        parsed = json.loads(raw)
    except ValueError:
        parsed = raw.decode(errors="replace")
    return resp.status, resp.headers, parsed


def check(name, ok, detail=""):
    ok = bool(ok)  # an empty list or None must fail, not slip past the final count
    results.append(ok)
    print(("PASS  " if ok else "FAIL  ") + name + ("" if ok else f"   -> {detail}"))


PEOPLE = [
    {"name": "Ada", "age": 36, "city": "London"},
    {"name": "Linus", "age": 55, "city": "Helsinki"},
    {"name": "Grace", "age": 85, "city": "New York"},
]


def body(mode, query, **extra):
    return {"mode": mode, "collection": "people", "collections": {"people": PEOPLE}, "query": query, **extra}


# --- liveness and discovery -------------------------------------------------------------
s, _, b = call("GET", "/healthz")
check("healthz", s == 200 and b == "ok", (s, b))
s, _, b = call("GET", "/readyz")
check("readyz (reaches MongoDB)", s == 200, (s, b))
s, _, limits = call("GET", "/api/v1/limits")
check("limits: 8 collections, 256 docs, no result cap",
      s == 200 and limits.get("maxCollections") == 8 and limits.get("maxDocsPerCollection") == 256
      and limits.get("maxResults") is None and limits.get("maxTimeMs") == 10000, limits)
s, _, spec = call("GET", "/api/v1/openapi.json")
check("openapi.json is 3.1 and lists /api/v1/run",
      s == 200 and str(spec.get("openapi", "")).startswith("3.1") and "/api/v1/run" in spec.get("paths", {}), s)
s, _, spec2 = call("GET", "/openapi.json")
check("root alias /openapi.json", s == 200 and spec2 == spec, s)
s, _, txt = call("GET", "/llms.txt")
check("llms.txt", s == 200 and "MongoDB Playground" in txt and "/api/v1/run" in txt, s)
s, _, examples = call("GET", "/api/v1/examples")
check("examples (>= 14)", s == 200 and isinstance(examples, list) and len(examples) >= 14, s)

# --- templates and discovery -------------------------------------------------------------
tpl = {e["id"]: e for e in examples} if isinstance(examples, list) else {}
check("single and multi templates exist and share the per-collection shape",
      tpl.get("single", {}).get("kind") == "template" and tpl.get("multi", {}).get("kind") == "template"
      and "documents:" in tpl["single"]["config"] and "documents:" in tpl["multi"]["config"], list(tpl))
s, _, g = call("GET", "/api/v1/guide")
opts = [o["id"] for o in g.get("options", [])] if isinstance(g, dict) else []
check("setup guide lists every way to run it", s == 200 and opts == ["hosted", "mcp", "atlas-local", "atlas-free", "atlas-ephemeral", "docker", "desktop"], (s, opts))
check("guide has the Atlas sign-up link with UTMs, no placeholders left",
      "utm_source=mongoplayground" in str(g) and "utm_medium=referral" in str(g) and "utm_campaign=playground-free-cluster" in str(g) and "{origin}" not in str(g) and "{atlasRegister}" not in str(g), str(g)[:300])
s, _, card = call("GET", "/.well-known/mcp.json")
check("MCP discovery card", s == 200 and card.get("transport") == "streamable-http" and card["url"].endswith("/mcp") and "get_setup_guide" in card["tools"], (s, card))

# --- export, rate limit headers, feature flags --------------------------------------------------
exp_req = {"script": "db.people.find({age: {$gt: 30}})", "config": "db = { people: { documents: [{name: 'Ada', age: 36, _id: ObjectId('507f1f77bcf86cd799439011')}], indexes: [{key: {age: 1}}] } }"}
s, _, b = call("POST", "/api/v1/export", dict(exp_req, format="mongosh"))
check("export mongosh script recreates data, indexes and statements",
      s == 200 and b["filename"] == "playground.js" and "insertMany" in b["content"] and "createIndex({ age: 1 })" in b["content"]
      and 'ObjectId("507f1f77bcf86cd799439011")' in b["content"] and "db.people.find({age: {$gt: 30}})" in b["content"], (s, b))
s, _, b = call("POST", "/api/v1/export", dict(exp_req, format="json"))
check("export json keeps extended json", s == 200 and json.loads(b["content"])["collections"]["people"]["documents"][0]["_id"] == {"$oid": "507f1f77bcf86cd799439011"}, (s, b))
s, _, b = call("POST", "/api/v1/export", dict(exp_req, format="xml"))
check("export rejects unknown formats", s == 422, (s, b))
s, h, b = call("POST", "/api/v1/validate", body("find", {"filter": {}}))
check("responses carry rate-limit headers", h.get("X-RateLimit-Limit") is not None and h.get("X-RateLimit-Remaining") is not None, dict(h))
check("limits report rate limits, turnstile and feature flags", "rateLimit" in limits and "turnstile" in limits and "sampleData" in limits.get("features", {}) and limits.get("deployment", {}).get("cluster"), limits)
s, _, b = call("POST", "/api/v1/verify", {})
check("verify is a no-op without Turnstile (or asks for a token with it)", (s == 200 and b.get("verificationRequired") is False) or s == 422, (s, b))

# --- validate ---------------------------------------------------------------------------
s, _, b = call("POST", "/api/v1/validate", body("find", {"filter": {"age": {"$gt": 40}}}))
check("validate ok", s == 200 and b.get("valid") is True and b["documentCounts"]["people"] == 3, (s, b))
s, _, b = call("POST", "/api/v1/validate", body("find", {"filter": {"$where": "1"}}))
err = b.get("error", {}) if isinstance(b, dict) else {}
check("validate rejects $where with a hint", s == 403 and err.get("code") == "forbidden" and err.get("hint"), (s, b))

# --- every mode -------------------------------------------------------------------------
s, _, b = call("POST", "/api/v1/run", body("find", {"filter": {"age": {"$gt": 60}}, "projection": {"_id": 0, "name": 1}}))
check("find", s == 200 and b["results"] == [{"name": "Grace"}], (s, b))
s, _, b = call("POST", "/api/v1/run", body("aggregate", [{"$group": {"_id": None, "n": {"$sum": 1}}}]))
check("aggregate", s == 200 and b["results"] == [{"_id": None, "n": 3}], (s, b))
s, _, b = call("POST", "/api/v1/run", body("count", {"filter": {"age": {"$gt": 40}}}))
check("count", s == 200 and b["results"] == [{"count": 2}], (s, b))
s, _, b = call("POST", "/api/v1/run", body("distinct", {"key": "city"}))
check("distinct", s == 200 and sorted(b["results"]) == ["Helsinki", "London", "New York"], (s, b))
s, _, b = call("POST", "/api/v1/run", body("insert", {"documents": [{"name": "Tim"}]}))
check("insert", s == 200 and b["write"] == {"inserted": 1} and len(b["results"]) == 4, (s, b))
s, _, b = call("POST", "/api/v1/run", body("update", {"filter": {"name": "Ada"}, "update": {"$set": {"age": 37}}}))
check("update", s == 200 and b["write"].get("modified") == 1, (s, b))
s, _, b = call("POST", "/api/v1/run", body("update", {"filter": {"name": "Zed"}, "update": {"$set": {"age": 1}}, "upsert": True}))
check("update upsert", s == 200 and b["write"].get("upserted") == 1, (s, b))
s, _, b = call("POST", "/api/v1/run", body("delete", {"filter": {}, "multi": True}))
check("delete many", s == 200 and b["write"] == {"deleted": 3} and b["results"] == [], (s, b))
s, _, b = call("POST", "/api/v1/run", body("find", {"filter": {"age": {"$nope": 1}}}))
check("bad operator is a database_error with a hint",
      s == 422 and b["error"]["code"] == "database_error" and b["error"]["hint"], (s, b))

# --- every shipped example (templates included) runs as a script ---------------------------
for ex in examples if isinstance(examples, list) else []:
    req = {k: ex[k] for k in ("script", "config", "datasource") if k in ex}
    needs_search = "searchIndexes" in ex.get("config", "") or "vectorSearchIndexes" in ex.get("config", "") or "createSearchIndex" in ex["script"]
    if needs_search and not limits.get("searchSupported"):
        print(f"SKIP  example {ex['id']} (this deployment has no Atlas Search)")
        continue
    if "datasource" in req and not limits.get("sampleSupported"):
        print(f"SKIP  example {ex['id']} (no sample data loaded)")
        continue
    s, _, b = call("POST", "/api/v1/run", req)
    check(f"example {ex['id']} runs", s == 200 and b.get("ok") is True and b.get("statements"), (s, b))
    s, _, b = call("POST", "/api/v1/explain", req)
    check(f"example {ex['id']} explains", s == 200 and b.get("ok") is True, (s, b))

# --- explain ----------------------------------------------------------------------------
s, _, b = call("POST", "/api/v1/explain", body("find", {"filter": {"age": {"$gt": 40}}}))
summ = b.get("explain", {}).get("summary", {}) if isinstance(b, dict) else {}
check("explain reports COLLSCAN with advice",
      s == 200 and b["results"] == [] and summ.get("collscan") is True and summ.get("returned") == 2 and summ.get("advice"), (s, b))
s, _, b = call("POST", "/api/v1/run", body("find", {"filter": {}}, explain="queryPlanner"))
check("run with explain returns results and a plan", s == 200 and len(b["results"]) == 3 and b["explain"]["summary"]["verbosity"] == "queryPlanner", (s, b))
s, _, b = call("POST", "/api/v1/run", body("insert", {"document": {"a": 1}}, explain="executionStats"))
check("insert + explain is rejected", s == 422 and b["error"]["code"] == "invalid_request", (s, b))
s, _, b = call("POST", "/api/v1/run", body("delete", {"filter": {}, "multi": True}, explain="executionStats"))
check("explaining a delete does not delete before the real delete reports 3",
      s == 200 and b["write"] == {"deleted": 3} and b["explain"]["summary"]["stages"], (s, b))

# --- regex literals ----------------------------------------------------------------------
text = {"mode": "find", "collection": "people",
        "collections": json.dumps({"people": [{"name": "Ada"}, {"name": "ada lovelace"}, {"name": "Linus"}]}) + " // inline comment",
        "query": '{"filter": {"name": /^ada/i}, "sort": {"name": 1}}  /* literal */'}
s, _, b = call("POST", "/api/v1/run", text)
check("regex literal in relaxed-JSON text", s == 200 and [d["name"] for d in b["results"]] == ["Ada", "ada lovelace"], (s, b))
s, _, b = call("POST", "/api/v1/run", dict(text, query='{"filter": {"name": /x/g}}'))
check("bad regex flag -> invalid_request naming the flag", s == 422 and "`g`" in b["error"]["message"] and b["error"]["hint"], (s, b))
s, _, b = call("POST", "/api/v1/run", dict(text, query='{"filter": {"name": /abc}}'))
check("unterminated regex -> structured error with a line", s == 422 and "line" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/run", body("find", {"filter": {"name": {"$regex": "^a", "$options": "i"}}}))
check("$regex operator form", s == 200 and [d["name"] for d in b["results"]] == ["Ada"], (s, b))
s, _, b = call("POST", "/api/v1/run", "not json at all")
check("malformed body -> structured error (not plain text)", s == 422 and isinstance(b, dict) and b["error"]["code"] == "invalid_request", (s, b))

# --- indexes ----------------------------------------------------------------------------
idx = {"people": [{"key": {"age": 1}}]}
s, _, b = call("POST", "/api/v1/explain", body("find", {"filter": {"age": 36}}, indexes=idx))
summ = b.get("explain", {}).get("summary", {}) if isinstance(b, dict) else {}
check("index turns COLLSCAN into IXSCAN", s == 200 and summ.get("indexUsed") == "age_1" and summ.get("collscan") is False, (s, summ))
s, _, b = call("POST", "/api/v1/run", body("listIndexes", {}, indexes={"people": [{"key": {"age": 1}}, {"key": {"city": 1}, "options": {"name": "by_city"}}]}))
check("listIndexes", s == 200 and sorted(i["name"] for i in b["results"]) == ["_id_", "age_1", "by_city"], (s, b))
four = {"people": [{"key": {f: 1}} for f in "abcd"]}
s, _, b = call("POST", "/api/v1/run", body("find", {}, indexes=four))
check("4th index on a collection is rejected", s == 422 and "limit is 3" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/run", body("find", {}, indexes={"people": [{"key": {"a": 1}, "options": {"storageEngine": {}}}]}))
check("disallowed index option is rejected", s == 422 and "not allowed" in b["error"]["message"], (s, b))
dup = {"mode": "find", "collection": "p", "collections": {"p": [{"city": "A"}, {"city": "A"}]}, "query": {"filter": {}},
       "indexes": {"p": [{"key": {"city": 1}, "options": {"unique": True}}]}}
s, _, b = call("POST", "/api/v1/run", dup)
check("unique index over duplicates -> database_error", s == 422 and b["error"]["code"] == "database_error", (s, b))
s, _, limits = call("GET", "/api/v1/limits")
check("limits report index and regex limits", limits.get("maxIndexesPerCollection") == 3 and limits.get("maxRegexPatternLength") == 256, limits)

# --- share links ------------------------------------------------------------------------
shared = {"mode": "find", "collection": "people",
          "collections": '{"people": [{"name": "Ada"}]}',
          "query": '{"filter": {"name": /^a/i}}  // typed as relaxed JSON'}
s, _, a = call("POST", "/api/v1/share", shared)
check("share returns an id and url", s == 200 and a.get("id") and a.get("url") == "/p/" + a.get("id", ""), (s, a))
sid = a.get("id", "x")
s, _, b = call("POST", "/api/v1/share", shared)
check("every share gets its own UUID", s == 200 and b.get("id") != sid and len(b.get("id", "")) == 36, (s, b))
s, _, b = call("POST", "/api/v1/share", dict(shared, collection="other_name"))
check("a different playground is still rejected or shared", s == 422 or (s == 200 and b.get("id") != sid), (s, b))
s, h, b = call("GET", f"/api/v1/p/{sid}")
check("load share round-trips the text exactly", s == 200 and b.get("query") == shared["query"] and b.get("collections") == shared["collections"], (s, b))
check("shares are cacheable forever", "immutable" in (h.get("Cache-Control") or ""), dict(h))
s, _, b = call("POST", "/api/v1/run", {k: b[k] for k in ("mode", "collection", "collections", "query")})
check("a loaded share runs as-is", s == 200 and [d["name"] for d in b["results"]] == ["Ada"], (s, b))
s, _, b = call("GET", "/api/v1/p/doesnotexist")
check("unknown share -> 404 not_found", s == 404 and b["error"]["code"] == "not_found" and b["error"]["hint"], (s, b))
s, _, b = call("POST", "/api/v1/share", dict(shared, query={"filter": {"$where": "1"}}))
check("an invalid playground cannot be shared", s == 403 and b["error"]["code"] == "forbidden", (s, b))

# --- stats events (accepted, never echoed) ----------------------------------------------------
s, _, _ = call("POST", "/api/v1/events", {"type": "page_view"}, headers={"X-Session-Id": "smoke-test", "X-Client": "web"})
check("ui event accepted", s == 204, s)
s, _, b = call("POST", "/api/v1/events", {"type": "drop_tables"})
check("unknown ui event type rejected", s == 422 and b["error"]["code"] == "invalid_request", (s, b))

# --- sample data ------------------------------------------------------------------------------
s, _, b = call("GET", "/api/v1/samples")
check("samples listing", s == 200 and "available" in b and isinstance(b["databases"], list), (s, b))
if not limits.get("features", {}).get("sampleData"):
    sample_run = {"script": "db.movies.find()", "datasource": {"sample": "sample_mflix"}}
    s, _, b = call("POST", "/api/v1/run", sample_run)
    check("sample data is off by default: datasource is rejected clearly", s == 422 and "turned off" in b["error"]["message"] and b["error"]["hint"], (s, b))
    s, _, b = call("GET", "/api/v1/samples/sample_mflix/movies")
    check("sample preview is 404 when the flag is off", s == 404, (s, b))
if limits.get("sampleSupported") and b.get("databases"):
    d = b["databases"][0]
    s, _, p = call("GET", f"/api/v1/samples/{d['name']}/{d['collections'][0]}")
    check("sample preview returns documents", s == 200 and len(p["documents"]) >= 1, (s, p))
elif limits.get("features", {}).get("sampleData"):
    sample_run = {"mode": "find", "collection": "movies", "datasource": {"sample": "sample_mflix"}, "query": {}}
    s, _, b = call("POST", "/api/v1/run", sample_run)
    check("sample run without sample data -> clear error", s == 422 and "unknown sample database" in b["error"]["message"], (s, b))
if limits.get("features", {}).get("sampleData"):
    s, _, b = call("POST", "/api/v1/run", {"mode": "insert", "collection": "movies", "datasource": {"sample": "sample_mflix"}, "query": {"document": {}}})
    check("sample data is read-only", s == 422 and "read-only" in b["error"]["message"], (s, b))

# --- search index -----------------------------------------------------------------------------
search_req = {"mode": "aggregate", "collection": "movies",
              "collections": {"movies": [{"title": "The Matrix", "plot": "a hacker learns reality is simulated"}, {"title": "Alien", "plot": "a deadly creature in space"}]},
              "searchIndexes": {"movies": {"definition": {"mappings": {"dynamic": True}}}},
              "query": [{"$search": {"text": {"query": "hacker", "path": "plot"}}}, {"$project": {"_id": 0, "title": 1}}]}
s, _, b = call("POST", "/api/v1/run", search_req)
if limits.get("searchSupported"):
    check("$search finds the document after the index builds", s == 200 and b["results"] == [{"title": "The Matrix"}], (s, b))
else:
    check("no Atlas Search here -> clear error", s == 422 and "does not support Atlas Search" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/validate", dict(search_req, searchIndexes={}))
check("$search without a search index is rejected", s == 422 and "searchIndexes" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/validate", dict(search_req, searchIndexes={"movies": {"definition": {"fields": []}}}))
check("vector-style definitions are rejected", s == 422, (s, b))

# --- internal jobs are hidden ----------------------------------------------------------------
s, _, _ = call("POST", "/internal/jobs/sweep")
check("internal jobs 404 without a token", s == 404, s)

# --- shell-style scripts: several statements, shell syntax, one transcript -----------------------
S = {"config": "db = { people: [ {name: 'Ada', age: 36, city: 'London'}, {name: 'Linus', age: 55, city: 'Helsinki'} ] }"}
s, _, b = call("POST", "/api/v1/run", dict(S, script="""
    db.people.insertMany([{name: 'Tim', age: 30, city: 'London'}, {name: 'Bo', age: 41, city: 'Boston'}])
    db.people.updateMany({age: {$gt: 35}}, {$set: {senior: true}})   // unquoted keys, comments
    db.people.find({senior: true}, {_id: 0, name: 1}).sort({name: 1})
    db.people.deleteOne({name: 'Tim'})
    db.people.countDocuments({})
    db.people.distinct('city')
"""))
ops = [t["op"] for t in b.get("statements", [])] if isinstance(b, dict) else []
check("a CRUD script runs all its statements in order", s == 200 and b.get("ok") and ops == ["insert", "update", "find", "delete", "count", "distinct"], (s, ops))
check("the transcript has each result", b["statements"][2]["results"] == [{"name": "Ada"}, {"name": "Bo"}, {"name": "Linus"}] and b["statements"][4]["results"] == [{"count": 3}], b)
check("statements echo their text and line", b["statements"][2]["statement"].startswith("db.people.find") and b["statements"][2]["line"] == 4, b["statements"][2])
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.countDocuments({}); db.people.find({age: {$nope: 1}}); db.people.countDocuments({})"))
check("a failing statement stops the script, earlier output kept", s == 200 and b["ok"] is False and len(b["statements"]) == 2 and b["error"]["code"] == "database_error" and b["error"]["hint"], (s, b))
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.find({}).sort({age: 1})\ndb.people.bogus({})"))
check("an unknown method is rejected with the statement and line", s == 422 and "statement 2" in b["error"]["message"] and "line 2" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.find({$where: 'sleep(1000)'})"))
check("forbidden operators are rejected in scripts too", s == 403 and b["error"]["code"] == "forbidden", (s, b))
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.find({name: /^a/i}, {_id: 0}); db.people.find({_id: ObjectId('507f1f77bcf86cd799439011')})"))
check("regex literals and ObjectId() work in scripts", s == 200 and b["ok"] and [d["name"] for d in b["statements"][0]["results"]] == ["Ada"] and "_id" not in b["statements"][0]["results"][0], (s, b))
s, _, b = call("POST", "/api/v1/explain", dict(S, script="db.people.find({age: 36}); db.people.insertOne({age: 1}); db.people.deleteMany({})"))
check("explain mode explains reads/writes but still runs inserts", s == 200 and b["ok"] and b["statements"][0]["explain"] and b["statements"][1]["write"]["inserted"] == 1 and b["statements"][2]["explain"], (s, b))
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.find({age: 36}).explain()"))
check("find().explain() returns a plan, no documents", s == 200 and b["statements"][0]["explain"]["summary"]["collscan"] is True and b["statements"][0]["results"] == [], (s, b))
single = {"config": "[{n: 1}, {n: 2}]", "script": "db.collection.find({n: {$gt: 1}})"}
s, _, b = call("POST", "/api/v1/run", single)
check("single-collection template: an array is the collection named `collection`", s == 200 and b["statements"][0]["results"] and b["statements"][0]["results"][0]["n"] == 2, (s, b))
panel = {"config": "db = { people: { documents: [{a: 1}, {a: 2}], indexes: [{key: {a: 1}}] } }", "script": "db.people.find({a: 2}).explain(); db.people.getIndexes()"}
s, _, b = call("POST", "/api/v1/run", panel)
check("per-collection data panel: documents and indexes in one place", s == 200 and b["ok"] and b["statements"][0]["explain"]["summary"]["indexUsed"] == "a_1" and len(b["statements"][1]["results"]) == 2, (s, b))
s, _, b = call("POST", "/api/v1/run", dict(S, script="db.people.createIndex({a: 1}); db.people.createIndex({b: 1}); db.people.createIndex({c: 1}); db.people.createIndex({d: 1})"))
check("4th index in a script is rejected", s == 422 and "limit is 3" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/validate", dict(S, script="db.people.find(); db.people.insertOne({a: 1})"))
check("validate lists the statements without running them", s == 200 and b["valid"] and [t["op"] for t in b["statements"]] == ["find", "insert"], (s, b))
s, _, b = call("POST", "/api/v1/run", {"script": "db.people.find()", "config": "{people: [}"})
check("a data-panel syntax error is a structured 422 with a line", s == 422 and "line 1" in b["error"]["message"], (s, b))
s, _, b = call("POST", "/api/v1/share", dict(S, script="db.people.find({name: /^a/i})  // shared"))
sid2 = b.get("id") if s == 200 else None
s, _, b = call("GET", f"/api/v1/p/{sid2}")
check("a script playground can be shared and loaded back as typed", s == 200 and b.get("script", "").endswith("// shared") and "people" in str(b.get("collections")), (s, b))

# --- no result cap ----------------------------------------------------------------------
big = {"mode": "find", "collection": "n", "collections": {"n": [{"i": i} for i in range(256)]}, "query": {"filter": {}}}
s, _, b = call("POST", "/api/v1/run", big)
check("256 results, more than one cursor batch", s == 200 and len(b["results"]) == 256, (s, len(b.get("results", []))))
over = dict(big, collections={"n": [{"i": i} for i in range(257)]})
s, _, b = call("POST", "/api/v1/run", over)
check("257 documents is over the limit", s == 422 and b["error"]["code"] == "invalid_request", (s, b))

# --- compression ------------------------------------------------------------------------
s, _, b = call("POST", "/api/v1/run", big, compress=True)
check("gzip-compressed request body", s == 200 and len(b["results"]) == 256, (s, b))
s, h, b = call("POST", "/api/v1/run", big, headers={"Accept-Encoding": "gzip"})
check("gzip response when asked", s == 200 and h.get("Content-Encoding") == "gzip" and len(b["results"]) == 256, (s, dict(h)))

# --- MCP --------------------------------------------------------------------------------
def rpc(method, params=None, id=1, path="/api/v1/mcp"):
    return call("POST", path, {"jsonrpc": "2.0", "id": id, "method": method, "params": params or {}})


s, _, b = rpc("initialize", {"protocolVersion": "2025-06-18"})
check("mcp initialize", s == 200 and b["result"]["serverInfo"]["name"] == "mongodb-playground", (s, b))
s, _, b = rpc("tools/list")
names = [t["name"] for t in b["result"]["tools"]] if s == 200 else []
check("mcp tools/list", names == ["run_query", "validate_query", "explain_query", "share_query", "get_shared", "export_playground", "list_examples", "get_setup_guide", "get_limits"], names)
s, _, b = rpc("tools/call", {"name": "run_query", "arguments": body("count", {"filter": {}})})
check("mcp run_query (structured)", s == 200 and b["result"]["isError"] is False and b["result"]["structuredContent"]["results"] == [{"count": 3}], (s, b))
s, _, b = rpc("tools/call", {"name": "run_query", "arguments": dict(S, script="db.people.insertOne({name: 'X'}); db.people.countDocuments({})")})
check("mcp run_query (script transcript)", s == 200 and b["result"]["isError"] is False and b["result"]["structuredContent"]["statements"][1]["results"] == [{"count": 3}], (s, b))
s, _, b = rpc("tools/call", {"name": "run_query", "arguments": dict(S, script="db.people.find({a: {$nope: 1}})")})
check("mcp run_query: a failing statement -> isError with the transcript", s == 200 and b["result"]["isError"] is True and b["result"]["structuredContent"]["ok"] is False, (s, b))
s, _, b = rpc("tools/call", {"name": "explain_query", "arguments": body("find", {"filter": {}})})
check("mcp explain_query", s == 200 and b["result"]["isError"] is False and b["result"]["structuredContent"]["explain"]["summary"]["stages"], (s, b))
s, _, b = rpc("tools/call", {"name": "validate_query", "arguments": body("find", {"filter": {"$where": "1"}})})
check("mcp validate_query forbidden -> isError + hint",
      s == 200 and b["result"]["isError"] is True and b["result"]["structuredContent"]["hint"], (s, b))
s, _, b = rpc("tools/call", {"name": "share_query", "arguments": shared})
mcp_id = b["result"]["structuredContent"].get("id") if s == 200 and not b["result"]["isError"] else None
check("mcp share_query returns a UUID", bool(mcp_id) and len(mcp_id) == 36, (s, b))
s, _, b = rpc("tools/call", {"name": "get_shared", "arguments": {"id": mcp_id or "x"}})
check("mcp get_shared", s == 200 and b["result"]["structuredContent"].get("query") == shared["query"], (s, b))
s, _, b = rpc("tools/call", {"name": "get_shared", "arguments": {"id": "nope"}})
check("mcp get_shared unknown id -> isError + hint", b["result"]["isError"] is True and b["result"]["structuredContent"]["code"] == "not_found", b)
s, _, b = rpc("tools/call", {"name": "get_setup_guide"})
check("mcp get_setup_guide", s == 200 and b["result"]["isError"] is False and "claude mcp add" in str(b["result"]["structuredContent"]), (s, b))
s, _, b = call("POST", "/mcp", {"jsonrpc": "2.0", "id": 2, "method": "ping"})
check("mcp alias /mcp", s == 200 and b["result"] == {}, (s, b))
s, _, _ = call("POST", "/api/v1/mcp", {"jsonrpc": "2.0", "method": "notifications/initialized"})
check("mcp notification -> 202", s == 202, s)
s, _, _ = call("GET", "/api/v1/mcp")
check("mcp GET -> 405", s == 405, s)

failed = results.count(False)
print(f"\n{results.count(True)} passed, {failed} failed")
sys.exit(1 if failed else 0)
