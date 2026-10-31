# Failure analysis golden cases

This is a **manual, live-model evaluation runbook** for the failure investigation
feature (design §12.4). It is **not part of CI** and nothing in this document is
a recorded result: run the cases against a real console, real OpenObserve data
and a real model, then fill in the report template yourself.

CI already pins the deterministic behaviour (see
[Deterministic coverage](#deterministic-coverage)); what it cannot pin is whether
the model picks the right cause from real telemetry, cites only real evidence and
finishes inside the budget. That is what these eight cases measure.

## Prerequisites

- A real console (not `CONSOLE_DEMO=1`): `OPENOBSERVE_BASE_URL`,
  `OPENOBSERVE_ORG` and credentials set; at least one doggy-dns instance
  reachable from the console through its control-plane MCP endpoint.
- A non-empty `MODEL_API_KEY` (and `MODEL_NAME`) for an OpenAI-compatible
  endpoint that supports tool calling. The model name is part of the cache key,
  so keep it fixed for the whole run.
- `CONSOLE_ANALYSIS_ENABLED=true`. Defaults: 10 starts/minute, 60 s deadline,
  6 drill-down calls, 30 day retention. Leave them at the defaults unless a case
  says otherwise; record any change.
- OpenObserve MCP enabled if you want drill-down. Without it the analysis is
  evidence-only (`NoDrilldown`); cases still run, but tool-call metrics are 0.
- Ability to create the evidence conditions: a resolvable upstream that times
  out, a closed upstream port, a policy plugin that refuses, a stopped
  doggy-dns process, a failing trace stream, and a trace with more than 200 log
  rows. If a condition cannot be produced, mark the case **not run**, not
  passed.
- All calls below assume the console on `http://127.0.0.1:8787`. Use
  `jq` to capture snapshots.

## Workflow

1. Start the console: `cargo run` (real mode).
2. Pick a failure trace from the failure list. Open the drawer, or use
   `/failures/{traceId}?windowMinutes=60`. Opening the drawer loads evidence and
   then starts (or reuses) an analysis automatically.
3. Capture the evidence snapshot **before** judging the verdict:

   ```bash
   curl -s 'http://127.0.0.1:8787/api/failures/<traceId>/evidence?windowMinutes=60' \
     | tee evidence-<case>.json | jq '{
         traceId, filter, fingerprint, missing,
         group: {query: .group.query, qtype: .group.qtype,
                 responseCode: .group.responseCode, error: .group.error,
                 authority: .group.authority, instance: .group.instance},
         instance: (.instance | if . then {matchQuality, reachable, missing}
                    else null end),
         logs: {total: .logs.total, kept: (.logs.items|length),
                truncated: .logs.truncated, omitted: .logs.omitted},
         spans: {total: .spans.total, kept: (.spans.items|length),
                 truncated: .spans.truncated},
         metrics: (.metrics != null)
       }'
   ```

   Record the fingerprint, `missing`, `instance.reachable`/`matchQuality` and the
   log/span counts. These define the case.

4. Run the case from the drawer (auto-analysis or "重新分析") or through the API:

   ```bash
   curl -s -i -X POST \
     'http://127.0.0.1:8787/api/failures/<traceId>/analysis?windowMinutes=60&force=true'
   # 200 = cache hit (a previous attempt for this fingerprint/model/prompt);
   # 202 = new or in-flight job, body {"analysisId":"..."}
   ```

   Poll until terminal:

   ```bash
   curl -s 'http://127.0.0.1:8787/api/failures/<traceId>/analysis?analysis=<analysisId>' \
     | jq '{status, attempt, toolCallCount, startedAt, finishedAt,
            verdictCategory, confidence, promptVersion, model,
            resultStatus: .result.status, result, error, toolCalls}'
   ```

5. Verify citations. Every `findings[].evidenceIds` entry must be an ID present
   in the captured evidence snapshot (or produced by a drill-down call in this
   attempt; the server enforces this before storing). Count findings by `kind`
   and mark any claim that is not supported by its cited IDs.
6. Fill one row of the [results template](#results-template). Use
   `force=true` for repeated runs of the same case and note which attempt you
   report.

If analysis answers `503`, the run is invalid: the service is disabled by
config, demo mode, an empty model key or an unopened store. If it answers `429`,
wait for the token bucket to refill (default one token every 6 s) and retry.

## Cases

Each case lists: the evidence condition, how to reach it in the console, the
expected outcome, the citation requirement and the failure signs. "Explicit
unavailable" means `insufficient_evidence` or `unavailable` with the gap listed
in `notDeterminable`; it is an acceptable outcome when the evidence genuinely
cannot support the case.

### 1. Upstream timeout

- **ID / expected category:** `upstream_timeout`.
- **Evidence condition:** the failed trace's log rows contain an upstream
  timeout (the resolver's upstream did not answer within its deadline), and the
  failure group's `error`/`authority` point at that upstream.
- **Reproduce:** point doggy-dns at a test upstream that accepts the query and
  never answers, send a query, open the resulting failure trace.
- **Expected:** `succeeded` job with result status `ok`, category
  `upstream_timeout`. `high` confidence only if the instance was reachable and at
  least one observed finding cites evidence; otherwise `medium`/`low`.
- **Mandatory citation:** at least one `log:` row (or `span:`) showing the
  timeout; name the upstream from `group.authority` and mark that inference as
  `inferred` unless a cited row states it. Do not cite a config or plugin ID you
  did not receive.
- **Failure signs:** category `upstream_unreachable`/`unknown` despite a timeout
  row; an `ok` verdict with no observed finding; a claim about the upstream's
  configuration with no cited config/plugin ID; instance claimed reachable when
  `instance.reachable` is false.

### 2. Connection refused

- **ID / expected category:** `upstream_refused`.
- **Evidence condition:** the failure log rows report a refused connection
  (`connection refused` / `ECONNREFUSED`) to the upstream, with no successful
  answer.
- **Reproduce:** stop the upstream resolver (leave the port closed), send a
  query, open the failure trace.
- **Expected:** `ok`, category `upstream_refused`.
- **Mandatory citation:** the `log:` row with the refusal, or the `span:` whose
  error status carries it. If the upstream address is only inferred from the
  authority string, mark that finding `inferred`.
- **Failure signs:** conflating refused with timeout or with
  `upstream_unreachable`; blaming policy or the client; `ok` without a cited
  refusal row; invented endpoint facts.

### 3. Policy REFUSED

- **ID / expected category:** `policy_refused`.
- **Evidence condition:** a failure group with `responseCode=REFUSED` and a log
  row (or plugin detail) from the policy/ACL plugin that names the blocked
  query, or the ordered plugin chain from a reachable instance that shows the
  policy plugin enabled.
- **Reproduce:** enable a policy/ACL rule that refuses a domain (or use a real
  trace already refused by policy), query the domain, open the failure trace.
- **Expected:** `ok`, category `policy_refused`.
- **Mandatory citation:** the refusal `log:` row and/or the
  `plugin:<instanceId>:<order>` ID. A plugin-caused claim must cite the plugin
  ID, not just the instance.
- **Failure signs:** blaming the upstream; a policy claim with no plugin/log
  citation; citing `plugin:` IDs when `instance` is `null` or
  `reachable=false` (those IDs do not exist in that bundle); `high` confidence
  without a reachable instance.

### 4. NXDOMAIN is never a failure cause

- **ID / expected behaviour:** validation rejects an NXDOMAIN attribution;
  NXDOMAIN-only traces are not failures at all.
- **Evidence condition / reproduce:** two checks.
  1. Request a window with `responseCode=NXDOMAIN` in the failure list, and ask
     the evidence endpoint for a trace whose only answer is NXDOMAIN. The list
     and detail must exclude it and evidence must answer `404`.
  2. Open a real failure (cases 1–3, 5–7) and check that the verdict is not
     attributed to NXDOMAIN. To exercise the rejection path deliberately, use a
     model/prompt variant that emits NXDOMAIN (or rely on CI validation tests);
     the first submission is rejected with `NXDOMAIN is not a failure cause`,
     and after two rejections the job is `invalid` with no result.
- **Expected:** no stored `ok` result mentions NXDOMAIN in the verdict summary,
  a finding claim or a next step; a genuinely NXDOMAIN-only trace produces no
  bundle and no analysis. A repaired result must state the real cause with real
  citations.
- **Mandatory citation:** whatever real failure evidence supports the verdict;
  NXDOMAIN must never appear as an evidence ID, category or next step.
- **Failure signs:** evidence returns a bundle for an NXDOMAIN trace; the
  failure list shows NXDOMAIN; a succeeded result's summary/findings/next steps
  contain NXDOMAIN (the server should have rejected it); the model treats a
  negative answer as an outage.

### 5. Instance unreachable

- **ID / expected marker:** `instance_unreachable`.
- **Evidence condition:** the trace's identity matches an instance but its
  control-plane probe fails. Evidence keeps the telemetry identity,
  `reachable=false`, no status/pid/uptime/listen address, no plugins, no config,
  no runtime, and both the bundle and the instance list
  `instance_unreachable`. `matchQuality` is preserved when the instance was
  matched and then found unreachable, and is `none` when resolution itself
  failed.
- **Reproduce:** stop the doggy-dns process (or block its MCP port) after it has
  emitted telemetry for a failure, then open that failure trace.
- **Expected:** a verdict derived only from logs/spans/metrics — typically `ok`
  with a log-based category, or `insufficient_evidence`/`unavailable` with the
  gap listed. `high` confidence is impossible here: validation rejects it
  without a reachable instance.
- **Mandatory citation:** `log:`, `span:` or `metric:` IDs only. No config,
  plugin or runtime facts, because none are in the bundle.
- **Failure signs:** any claim about configuration, plugin order, runtime
  signals or process liveness; `high` confidence; citing a `config:` or
  `plugin:` ID; treating "no plugin evidence" as "no plugin is enabled".

### 6. Spans missing

- **ID / expected marker:** `spans_unavailable`.
- **Evidence condition:** the trace-stream query fails while logs and the
  instance are still available. The bundle keeps logs, instance and metrics,
  `spans.items` is empty and `spans_unavailable` is in `missing`. Assembly does
  not fail.
- **Reproduce:** make the trace stream temporarily unavailable (wrong trace
  stream / blocked trace query), or use a trace whose span search fails, then
  open the failure.
- **Expected:** analysis still runs on logs/instance. The verdict is `ok` if the
  logs support a category, otherwise `insufficient_evidence`; either way the
  span gap must be listed in `notDeterminable`.
- **Mandatory citation:** log/instance/metric IDs; no `span:` IDs (none exist).
- **Failure signs:** the evidence request itself errors; the analysis refuses to
  produce anything and does not say why; claims about span duration/status;
  `notDeterminable` does not mention the missing spans; the verdict is `high`
  confidence while the instance is also unreachable.

### 7. Logs truncated

- **ID / expected markers:** `logs_truncated` with an `omitted` count.
- **Evidence condition:** the trace has more than 200 log rows. Evidence keeps
  at most 200 (failure/WARN/ERROR first, first and last INFO, neighbours of
  failures), sets `logs.truncated=true`, reports `total` and `omitted`, and adds
  `logs_truncated` to `missing`.
- **Reproduce:** generate a chatty trace (>200 rows, e.g. debug logging or a
  repeated query loop) with at least one failure row, then open it.
- **Expected:** a verdict based on the selected rows plus an explicit
  `notDeterminable` entry that logs were truncated, including the omitted count
  when relevant. Never "no other errors occurred" or "all logs were checked".
- **Mandatory citation:** IDs of selected rows only; the omitted rows cannot be
  cited.
- **Failure signs:** claiming exhaustive log review or absence of errors;
  omitting the truncation gap; inventing a log ID; ignoring the failure row in
  favour of INFO chatter.

### 8. Fingerprint stale after evidence changes

- **ID / expected behaviour:** stale detection, never silent reuse.
- **Evidence condition / reproduce:** analyze a failure, then change its
  evidence (new logs for the same trace, a shifted window,
  `windowMinutes` change or a newly reachable instance). Open the share link
  `/failures/{traceId}?windowMinutes=<new>&analysis=<oldId>` and, separately,
  reload the drawer.
- **Expected:** the reloaded bundle has a different `fingerprint`; the drawer
  keeps the old analysis but marks it stale ("证据已变化，需重新分析") instead of
  presenting it as current. A new POST without `force` gets `202` (a new
  fingerprint is a cache miss), not `200`; "重新分析" creates attempt N+1 and the
  UI shows the new attempt. If the evidence is unchanged, the POST returns `200`
  and does not call the model. If the named attempt in the deep link no longer
  exists, the drawer falls back to a fresh analysis.
- **Mandatory citation:** not applicable; check instead that no stale result is
  displayed as current and that the new attempt records the new fingerprint.
- **Failure signs:** the old conclusion is shown as current after the fingerprint
  changes; a `200` response carrying the old fingerprint; no stale banner; no
  new attempt after reanalysis; a deep link restoring an attempt from another
  trace (must 404).

## Metrics

Measured over the eight cases, using the snapshot fields (`status`,
`verdictCategory`, `result.status`, `findings[].kind`, `findings[].evidenceIds`,
`toolCallCount`, `startedAt`, `finishedAt`):

| Metric | Target | How to measure |
| --- | --- | --- |
| Verdict accuracy | ≥ 80% (≥ 7 of 8) | A case is a hit when the stored verdict has the expected category with valid citations, or — for a case whose evidence cannot support a verdict — result status `insufficient_evidence`/`unavailable` with the expected gap in `notDeterminable` and no invented fact. Case 8 hits when the stale behaviour is observed. |
| Citation validity | 100% | Server-enforced: a succeeded result cannot contain an unknown ID. Additionally check every bundle ID by hand; findings citing tool-call IDs can only be trusted through the stored audit hash. |
| Unsupported-conclusion rate | ≤ 10% | Manually review claims: an `inferred` claim asserted as fact, or a next step that assumes unavailable evidence, counts as unsupported. |
| Average tool calls | ≤ 6 | Mean of `toolCallCount` over cases. The server budget makes > 6 impossible; a low mean is fine. |
| p95 analysis duration | ≤ 60 s | `finishedAt - startedAt` per case (includes queue time), nearest-rank p95. |
| Cost per analysis | report only | Provider spend divided by attempts actually billed; a `200` cache hit costs nothing. |
| Cache-hit rate | report only | `200` responses ÷ POST requests across the run. |

Record the model name, prompt version (`failure-analyst/v1`), console commit,
date and any non-default budgets with the results — the cache key and the
verdicts depend on them.

## Results template

Copy this table into the evaluation report. **This runbook contains no results;
the table is empty on purpose.**

| Case | Result status / category | Confidence | Observed IDs valid? | Tool calls | Duration (s) | Cache hit (200/202) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 upstream timeout | | | | | | | |
| 2 connection refused | | | | | | | |
| 3 policy REFUSED | | | | | | | |
| 4 NXDOMAIN exclusion | | | | | | | |
| 5 instance unreachable | | | | | | | |
| 6 spans missing | | | | | | | |
| 7 logs truncated | | | | | | | |
| 8 fingerprint stale | | | | | | | |

Also record: model, prompt version, commit, date, `CONSOLE_ANALYSIS_*` values,
average tool calls, p95 duration, total attempts, cache hits, approximate cost,
and every case marked not run with the reason.

## Deterministic coverage

These suites already run in CI. This runbook does not duplicate them; it covers
what they cannot: real telemetry, real model behaviour, live latency and cost.

| Guarantee | Suite |
| --- | --- |
| Evidence IDs, log hashing, log selection/truncation, fingerprint stability and sensitivity, bundle assembly (exact/host+ip/host/none, unreachable instance, spans missing, metrics missing) | `tests/evidence_test.rs` |
| Failure list and detail share one aggregate; detail 404 without a failure group; span-search failure marks spans unavailable | `tests/failures_test.rs` |
| Evidence endpoint: bundle shape, filter echo, 404 for unknown/non-failure traces, malformed query, upstream 502 | `tests/api_evidence_test.rs` |
| Result contract and validation: unknown IDs, observed without evidence, NXDOMAIN, gaps for `insufficient_evidence`, high confidence without reachable instance/observed finding | `tests/analysis_test.rs` (validation section) |
| Agent loop: direct submission, drill-down, 7th call refused, duplicate arguments refused, one repair then `invalid`, timeout → `unavailable`, 8 KB preview cap, tool audits | `tests/analysis_test.rs` (agent section) |
| Persistence: cache key, force/new attempt, one running job, attempt numbering, markers, retention prune, reopen, structured tool audits only | `tests/analysis_test.rs` (store section) |
| Analysis API: 202 job, 200 cache hit, 404 unknown trace/attempt, 429 rate limit, 503 without service, markers | `tests/api_analysis_test.rs` |
| Config defaults, minima and rejection of invalid analysis values | `tests/config_test.rs` |
| Prompt contract (NXDOMAIN rule, drill-down names, version) | `tests/prompt_test.rs`, `tests/analysis_test.rs` |
| UI state machine, polling, stale detection, deep-link restore, markers, drawer rendering/citation jump, chat handoff | `doggy-dns-ui/src/composables/__tests__/useInvestigation.spec.ts`, `.../useFailureList.spec.ts`, `.../components/__tests__/FailureDrawer.spec.ts`, `.../views/__tests__/FailureView.spec.ts`, `.../views/__tests__/OverviewView.spec.ts`, `.../router/__tests__/index.spec.ts` |

Not covered by CI, by design: real model verdict quality, real log/span content,
live tool availability, actual duration distribution and cost. Those are the
subject of this manual evaluation.
