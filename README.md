# Doggy DNS Console

An AI-first, read-only operations console for [doggy-dns](../doggy-dns). The dashboard reads
OpenObserve REST; the copilot uses OpenObserve MCP (allowlisted read-only tools) and the
doggy-dns instance MCP. No login: bind it to loopback by default.

## Quick start (no external services)

```bash
# terminal 1
CONSOLE_DEMO=1 cargo run

# terminal 2
cd doggy-dns-ui && bun install && bun run dev
# open http://localhost:5173
```

`CONSOLE_DEMO=1` uses built-in fixtures and a scripted model, so the overview, failure drawer,
and one AI turn work without OpenObserve, doggy-dns, or an API key. The production build can be
served by the Rust binary:

```bash
cd doggy-dns-ui && bun run build
cd .. && CONSOLE_DEMO=1 cargo run
# open http://127.0.0.1:8787
```

## Real deployment

1. Copy `.env.example` to `.env` and fill it in.
2. Set `OPENOBSERVE_BASE_URL`, `OPENOBSERVE_ORG`, and either `OPENOBSERVE_USER`/`OPENOBSERVE_PASSWORD` or `OPENOBSERVE_TOKEN`.
3. Set `MODEL_*` for any OpenAI-compatible endpoint. `MODEL_BASE_URL` falls back to
   `OPENAI_BASE_URL`, and `MODEL_API_KEY` falls back to `QWEN_API_KEY` then `OPENAI_API_KEY`.
4. Instances are discovered automatically from OpenObserve telemetry: doggy-dns
   exports `host.name`, `doggy_dns.ip_addr` and `doggy_dns.control.port`, which
   the console joins into `http://{ip_addr}:{control_port}/mcp`. Nodes that
   publish no address are probed at `http://{host}:{MCP_CONTROL_PORT}/mcp`
   (default `19090`). `doggy_dns.ip_addr` is the DNS listener's reachable
   address; if the control plane binds a different interface, list that node in
   `MCP_ENDPOINTS` instead. Set `MCP_TOKEN` to the control-plane token, if set.
5. Optional: set `MCP_ENDPOINTS` (comma separated) to probe extra or
   misconfigured nodes that never reach OpenObserve.
6. `cargo run`.

The console consumes all four read-only tools from each instance's control plane: `get_status`,
`list_plugins`, `get_config` and `get_runtime_status`. The instance drawer shows pid/status,
plugins, the redacted TOML (with the masked keys listed) and live signals such as Nacos cache
size/push freshness and XDP attachment. `MCP_TOKEN` is sent as a bearer token to every probed
endpoint — static and discovered alike — so anyone able to publish metrics for this OpenObserve
org can make the console present that token to an address of their choosing; keep the metric
stream trusted or leave `MCP_TOKEN` unset. Per-node tokens are not supported yet.

OpenObserve MCP is optional but recommended. If `/api/{org}/mcp` returns 404, enable it on the
OpenObserve instance (`O2_AI_ENABLED=true`, and `O2_TOOL_API_URL` where required). The console
only registers an allowlist: `StreamList`, `StreamSchema`, `SearchSQL`, `PrometheusQuery`,
`PrometheusRangeQuery`, `GetLatestTraces`, `GetTraceDAG`, `SearchAround`. It never exposes
`tool_search`, `tools_call`, or any write tool to the model. OpenObserve OSS has no granular
RBAC for MCP, so use a dedicated account and network restrictions.

Run the probes after upgrading OpenObserve:

```bash
cargo run --bin openobserve_probe       # REST streams, sample logs and metrics
cargo run --bin openobserve_mcp_probe  # MCP tools and allowlisted schemas
```

## Failure investigations

Opening a failure first loads a deterministic **evidence bundle** and then, when
analysis is available, starts a read-only forensic analysis of that bundle.
Evidence never depends on the model; analysis never blocks the evidence view.

### Evidence API

```
GET /api/failures/{traceId}/evidence?windowMinutes=&domain=&responseCode=
```

Synchronous, read-only and model-free. It uses the same window/domain/responseCode
filter as the failure list and detail, so the bundle cannot drift from the entry
that was clicked. The response contains:

- `group`: the same failure aggregate the list and detail return.
- `instance`: matched identity (`matchQuality` is `exact`, `host_ip`, `host` or
  `none`), `reachable`, status/pid/uptime/listen address, the ordered plugin
  chain, the redacted config and runtime signals — or `null` when no instance
  matched. An unreachable instance keeps only the telemetry identity.
- `logs`: up to 200 selected rows (failure/WARN/ERROR first, first and last
  INFO, immediate neighbours of failure rows) with `total`, `truncated` and
  `omitted`.
- `spans`: up to 200 spans with `total` and `truncated`.
- `metrics`: host and global query/failure counts for trace time ±15 minutes,
  or `null` when the metric series is unavailable.
- `fingerprint`: `sha256:` over the canonical evidence JSON. It excludes
  `generatedAt` and carries no model, prompt or tool metadata, so the same
  evidence always yields the same fingerprint.
- `missing`: `instance_unreachable`, `instance_not_found`, `spans_unavailable`,
  `metrics_unavailable`, `logs_truncated`.

Errors: `400` for a malformed query (the window is clamped to 5–1440 minutes,
so an out-of-range value is not an error), `404` when the trace is not a failure
group in that window, `502` when OpenObserve itself fails. Because a production
start still requires a model API key, evidence-only operation without one means
`CONSOLE_DEMO=1`; the endpoint itself never calls the model.

### Analysis API

```
POST /api/failures/{traceId}/analysis?windowMinutes=&domain=&responseCode=&force=
GET  /api/failures/{traceId}/analysis?analysis=<id>
GET  /api/analysis/markers?traceIds=<comma separated, max 200>
```

`POST` builds the evidence server-side and then:

- `200` with the stored snapshot when an attempt for the same evidence
  fingerprint, model name and prompt version (`failure-analyst/v1`) already
  **succeeded**. The model is not called.
- `202 { "analysisId": "…" }` for a newly queued job **or** when an attempt with
  the same key is already queued/running (the existing id is returned).
- `429` when the process-wide token bucket is exhausted. Cache hits and in-flight
  returns do not consume a token; only a new attempt does.
- `503` when no analysis service exists: `CONSOLE_ANALYSIS_ENABLED=false`,
  `CONSOLE_DEMO=1`, an empty model API key, or the investigation store could not
  be opened.
- `404` when the trace has no failure group, `502` when OpenObserve fails.

`force=true` skips the cache and creates a new attempt but never starts a second
concurrent job for the same key. A stale `running` row (older than twice the
deadline plus 60 s) is failed before a replacement starts.

`GET` returns the latest attempt for the trace, or the attempt named by
`?analysis=<id>` when it belongs to that trace; `404` when it is absent or
belongs to another trace. The markers
endpoint returns the latest lightweight marker per requested trace (no result
bodies) and is used only to badge the failure list.

### Attempts, cache and statuses

- Job status: `queued → running → succeeded | failed | invalid`.
- A `succeeded` job carries a conclusion status of `ok`,
  `insufficient_evidence` or `unavailable`. Two rejected submissions produce job
  status `invalid` with **no result**; its stored `error` is the raw submission
  payload capped at 512 bytes and kept for diagnosis — the validation reason is
  returned to the model as the tool reply but is not persisted as the error.
  `failed` means the attempt could not run to completion. Neither is cached, so
  a later POST starts a new attempt (rate limit permitting).
- The cache key is trace + evidence fingerprint + model name + prompt version.
  One queued/running job exists per key. Analyses run asynchronously and the UI
  polls once per second.

### Boundaries

- Drill-down is a server-enforced allowlist of eight read-only OpenObserve tools;
  the model never sees a write tool. Budget: at most 6 calls by default, a 60 s
  deadline and 8 KB per tool result preview. Repeating a tool with the same
  canonical arguments is refused.
- The final result must arrive through `submit_failure_analysis` and is validated
  server-side: every cited ID must exist in the bundle or a tool result, every
  `observed` finding must cite one, an `ok` result needs at least one observed
  finding, NXDOMAIN is rejected as a cause, `insufficient_evidence` must list
  gaps, and `high` confidence requires a reachable instance plus an observed
  finding. One repair attempt is allowed; a second invalid submission is
  discarded.
- Log messages, span attributes and tool output are treated as untrusted data;
  the prompt tells the model to ignore instructions found there. Model API keys
  are never logged.
- Persisted tool audits contain canonical arguments, duration, result hash and a
  structural digest (byte count, truncated flag, top-level keys or a truncated
  error message) — never raw telemetry rows.

### Storage and retention

`data/console.db` (or `CONSOLE_DB`) holds the agent sessions and a
`failure_investigations` table with `id` (primary key), trace id, `attempt`
(numbered per cache key: trace, evidence fingerprint, model and prompt
version), filter, evidence fingerprint, model, prompt version,
status, verdict category/confidence, result JSON, tool-call audit JSON, error
and timestamps. Raw logs and spans are **never** persisted. Retention defaults
to 30 days and is pruned at startup and once a day.

### Configuration

| Variable | Default | Minimum | Meaning |
| --- | --- | --- | --- |
| `CONSOLE_ANALYSIS_ENABLED` | `true` | — | Master switch; `false` makes the analysis endpoints answer `503` |
| `CONSOLE_ANALYSIS_RETENTION_DAYS` | `30` | `1` | Days to keep investigation rows |
| `CONSOLE_ANALYSIS_RATE_PER_MIN` | `10` | `1` | Process-wide analysis starts per minute |
| `CONSOLE_ANALYSIS_TIMEOUT_SECONDS` | `60` | `1` | Deadline for one analysis attempt |
| `CONSOLE_ANALYSIS_MAX_TOOL_CALLS` | `6` | `1` | Server-enforced drill-down budget |

Analysis also requires a model API key (`MODEL_API_KEY`, falling back to
`QWEN_API_KEY` then `OPENAI_API_KEY`), and is always unavailable in
`CONSOLE_DEMO=1`: the scripted demo model cannot submit a valid forensic result,
so the endpoints return `503` instead of persisting guaranteed-invalid attempts.

### UI

Opening a failure loads evidence and then analyzes it automatically; unchanged
evidence is served from the cache. The drawer shows the conclusion card
(category, confidence, summary, not-determinable gaps), observed and inferred
findings whose evidence IDs jump to and highlight the matching log/span/config/
plugin/metric row, tabs for logs, spans, instance and metrics, and a forensics
block (attempt, tool calls, fingerprint, model, prompt version) with a
reanalysis button. The chat handoff sends the trace id, fingerprint, conclusion
and analysis id to the chat session. When a reloaded bundle's fingerprint differs
from the analyzed one, the old analysis is kept but marked stale
("证据已变化，需重新分析" — evidence changed, reanalyze) and is never silently
reused.

A deep link `/failures/{traceId}?windowMinutes=…&analysis=<id>` restores the
window and the named attempt instead of starting a new one; an unknown id falls
back to a fresh analysis. The failure list shows a marker for traces that
already have an investigation and never triggers analysis itself.

The eight-case manual evaluation runbook is
[docs/analysis-golden-cases.md](docs/analysis-golden-cases.md).

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

cd doggy-dns-ui
bun run type-check
bun run test:unit -- --run
bun run build
bun run test:e2e
```

The frontend dev server proxies `/api` to `http://127.0.0.1:8787`. Playwright starts the demo
backend and the Vite dev server automatically.

## Boundaries

- Read-only: no config writes, no DNS mutations, no alert management.
- SQLite stores agent sessions and failure investigation metadata/results
  (`data/console.db`); raw telemetry logs and spans are never copied locally.
- Failure analysis is read-only (allowlisted OpenObserve tools only) and is
  disabled in demo mode or without a model API key.
- Success rate excludes `NXDOMAIN`; a window with zero queries shows `—`.
- `NXDOMAIN` is a normal negative answer, never a failure: the failure list and failure
  detail always exclude it, even when `responseCode=NXDOMAIN` is requested.
