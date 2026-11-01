# A1 live golden run — 2026-11-01

Execution record for [analysis-golden-cases.md](analysis-golden-cases.md) against real
telemetry and a real model. The runbook's Results template is intentionally empty; the
filled table below is this run's result.

## Run metadata

| Item | Value |
| --- | --- |
| Console commit | `669af08` — *feat: add evidence-first AI failure investigations (#5)* |
| Model | `qwen3.8-flash` (env `MODEL_NAME`, provider key from the shell profile, never inlined) |
| Prompt version | `failure-analyst/v1` |
| Date | 2026-11-01, 23:05–23:50 local |
| `CONSOLE_ANALYSIS_ENABLED` | `true` (default) |
| `CONSOLE_ANALYSIS_MAX_TOOL_CALLS` | `6` (default) |
| `CONSOLE_ANALYSIS_RATE_PER_MIN` | `10` (default) |
| `CONSOLE_ANALYSIS_TIMEOUT_SECONDS` | **non-default**: run started at the documented `60`, raised to `180`, then `300` (see blocker B2) |
| Telemetry | OpenObserve at `127.0.0.1:5080`, streams `doggy_dns` (logs + traces), read-only |
| Serving instance | local `doggy-dns` debug build with MCP enabled, `127.0.0.1:15353`, pid 96916, instance id `CherishCai-96916` |
| Failure factory | scripted UDP DNS upstream on `127.0.0.1:15354` with per-qname behaviour (silence / REFUSED / NXDOMAIN), harness config and scratch under `/tmp/doggy-live/` (outside both repos) |
| Case 4 check 2 | local OpenAI-compatible stub on `127.0.0.1:8899`, `MODEL_BASE_URL`/`MODEL_NAME` pointed at it (deployment change only, no code change) |

Nothing was written to OpenObserve or to the doggy-dns working tree; SQLite holds only
investigation rows, metadata and tool audits.

## Results

| Case | Result status / category | Confidence | Observed IDs valid? | Tool calls | Duration (s) | POST result (200/202) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 upstream timeout | `succeeded` / `upstream_timeout`, result `ok` | high | yes (all bundle IDs hand-checked) | 6 | 160 | 202 | trace `ef8366b0…`; four earlier attempts on this case were killed by the 60 s default and stored `unavailable`/`low`; forced re-run `0b80c071` 144 s and a second non-forced `36383c5f` 166 s both re-called the model (B1) |
| 2 connection refused | **not run** | — | — | — | — | — | The condition as written cannot be produced: with the upstream port closed doggy-dns logs `error=request timed out` + `rcode=ServFail`, not `connection refused`/`ECONNREFUSED` (macOS UDP gives no ICMP to hickory). Exploratory attempt `5dfbc44e` (trace `4e1c2fa3…`) returned `upstream_timeout`/high and explicitly refused to invent a refusal — honest, but counts as a miss. The case has since been restated around the RCODE, and the upstream-REFUSED attempt booked under case 3 (`84073c5b`) meets that restated condition — but it ran under the original spec, so this row stays `not run` and the 5/8 score is unchanged |
| 3 policy REFUSED | **not run** | — | — | — | — | — | No policy/ACL plugin exists in doggy-dns: the live instance exposes only `native`(order 0) and `forward`(order 1), and its builtin plugin set is exactly those two (a re-check of the workspace finds `policy`/`acl` only in comments, one of them about a *future* view/ACL-per-client handler), so the case has no reproducible condition. Exploratory upstream-REFUSED attempt `84073c5b` (trace `7c6863b7…`) returned `upstream_refused`/high with valid citations and did **not** blame a policy component — recorded as an observation, counts as a miss |
| 4 NXDOMAIN exclusion | `n/a` (rejection run stored `invalid`, `result = null`) | — | `n/a` | 0 | 0 | `202` (forced rejection run) | **Hit — both checks.** Check 1: an NXDOMAIN-only trace is absent from the failure list and detail, and evidence returns `404`. Check 2: the stub's submission was rejected on the first pass, the tool reply carried `{"error":"NXDOMAIN is not a failure cause"}`, exactly one repair attempt was sent, the stub repeated the answer, and the job stored `invalid` with `result = null`, `verdictCategory = null` and `error` = the capped raw submission. No stored result in the database mentions NXDOMAIN (0 of 14) |
| 5 instance unreachable | `succeeded` / `upstream_timeout`, result `ok` | medium | yes | 6 | 101 | 202, then **200** on the identical-evidence re-POST | doggy-dns stopped before the POST: `missing=["instance_unreachable"]`, `reachable=false`, `status/pid/uptimeSeconds/listenAddr` null, 0 plugins, no config, no runtime, `matchQuality` degraded from `exact` to `host`. Confidence `medium` and 7 `notDeterminable` entries, one of which is the exhausted budget. This is also the run that proves the cache works once the fingerprint is stable |
| 6 spans missing | `succeeded` / `upstream_timeout`, result `ok` | high | yes — no `span:` ID cited | 6 | 93 | 202 | bundle degraded exactly as specified: `missing=["spans_unavailable"]`, `logs=6`, `spans.items=[]`, assembly succeeded. `notDeterminable` names the span gap; no span-duration claim. High confidence is legitimate here because the instance was reachable. The first attempt of this case (`d0478a18`) died on the 180 s wall and stored `unavailable`/`low` |
| 7 logs truncated | **not run** | — | — | — | — | — | A single doggy-dns request yields ~6 log rows under one trace id, and a repeated-query loop creates *new* traces rather than a chatty one; producing >200 rows for one trace would require writing to OpenObserve, which is out of scope for a read-only round. Counts as a miss |
| 8 fingerprint stale | `succeeded` / `upstream_timeout`, result `ok`, then marked stale | high | yes | 6 | 156 | 202, then 202 on identical evidence (runbook expects 200 — see B1) | **Hit for stale detection, with the B1 caveat.** Live drawer at `/failures/{traceId}?windowMinutes=60&analysis=9a8f7059…` shows `已过期` + `证据已变化，需重新分析` + `重新分析` while keeping the old conclusion; a widened window produced a new fingerprint and a non-forced POST answered `202` with `attempt = 1` for the new key; a forced POST on an unchanged fingerprint answered `202` and stored `attempt = 2`; a deep link to another trace's attempt answered `404`. The "unchanged evidence → 200" clause only held while the instance was unreachable |

Scored verdicts: **5 of 8** — cases 1, 4, 5, 6, 8 hit; cases 2, 3, 7 are `not run` and
count as misses per the runbook's denominator.

## Metrics

| Metric | Target | Measured | Verdict |
| --- | --- | --- | --- |
| Verdict accuracy | ≥ 80% (≥ 7 of 8) | 5/8 = 62.5% (5/5 of the cases that could be produced) | **miss** |
| Citation validity | 100% | 100%. No result carries an unknown bundle ID (server-enforced and re-checked per case against the captured bundle). Two exploratory runs cite `tool:<n>:result`, which the offline check cannot resolve because tool payloads are not persisted — only canonical args, duration, result hash and structural summary | met |
| Unsupported-conclusion rate | ≤ 10% | 0 of 5 scored results asserted an unavailable fact as observed; instance/config/plugin claims were absent or marked `inferred`, and both unreachable-instance runs disclosed it | met |
| Average tool calls | ≤ 6 (budget recorded) | 6.0 over the four verdict-scored runs that called the model (cases 1, 5, 6, 8) — each exhausted the budget, and 2–4 of those 6 calls came back as server-side errors rather than data. Case 4's stub run made 0 calls and is excluded from this mean | at the bound, not usable headroom |
| p95 analysis duration | ≤ 60 s | 180 s nearest-rank over 14 model attempts (60, 60, 60, 60, 93, 101, 127, 144, 156, 160, 162, 166, 166, 180; mean 121 s) | **miss** |
| Cost per analysis | report only | Not measured — the harness has no provider billing readout. 14 billed `qwen3.8-flash` attempts, 5 of them deadline kills (four at the then-default 60 s, one at 180 s) plus 3 free stub attempts | reported |
| Cache-hit rate | report only | 1 of 11 non-forced POSTs returned `200` (9%). Of the three intentional identical-evidence repeats, only the one taken with the instance unreachable hit | reported (B1) |

## Blockers

**B1 — the evidence fingerprint is not stable while the matched instance is up.**
`compute_fingerprint` (`src/evidence/fingerprint.rs`) normalises only `generatedAt` and the
fingerprint field, so every evidence field participates — including
`instance.uptimeSeconds` and whatever `instance.runtime` snapshot the control-plane
`get_runtime_status` returns, which doggy-dns documents as sampled at call time.
Controlled A/B on one trace and one window:

* instance reachable — two consecutive evidence reads one second apart returned different
  fingerprints, the only payload difference being `uptimeSeconds 426 → 472`; the second
  non-forced POST therefore got `202` and re-called the model, and no two attempts ever
  share a key, so `attempt` stays 1 and "重新分析" cannot be observed to increment.
* instance stopped — fingerprint `sha256:be69f8a9…` reproduced identically across reads
  ~2 min apart, and the non-forced POST returned `200` with the completed snapshot.

This breaks the runbook's case-8 expectation ("if the evidence is unchanged, the POST
returns 200 and does not call the model") and, with it, the cache's whole purpose in
production, where the instance is by definition up whenever someone opens a failure. The
fix conflicts with existing assertions: `tests/evidence_test.rs:924` (`instance
uptimeSeconds`) and `:979` (`instance runtime`) currently require these fields to *change*
the fingerprint, and the design doc states "指纹：同证据稳定；…任一证据字段变化会变"
(`docs/superpowers/specs/2026-10-31-failure-evidence-investigation-design.md:295`). Any
change here is a spec ruling plus test-first work, not a drive-by patch.

**B2 — the documented 60 s default deadline is unreachable with this model.** Four
consecutive case-1 attempts at `CONSOLE_ANALYSIS_TIMEOUT_SECONDS=60` all terminated at
exactly 60 s as `succeeded` with result status `unavailable`, confidence `low` and the
error `model stream timed out`; one attempt at 180 s also timed out. Completed analyses took
93–166 s, i.e. roughly 15–27 s per model turn with the evidence payload plus 6 drill-downs.
A trivial prompt against the same endpoint answered in 1.16 s, so the endpoint is healthy and
the budget is the product, not the provider. Either the default rises or the deadline becomes
per-turn rather than whole-job.

## Secondary defects observed

* The OpenObserve drill-down budget is spent on call-shape errors: `openobserve_search_sql`
  rejected with *"parameter 'request_body' … is not of type 'object'"* when the model nests
  `agent_options`, `openobserve_trace_dag` rejected with *"unknown field
  'gen_ai_operation_name'"* and *"invalid parameter 'sort_order'"*, and *"Search stream not
  found: logs / traces / doggy-dns"*. The bundle does not carry the stream names it was
  assembled from, so the model has to guess them and burns calls doing so. Two cheap fixes:
  put the log/trace stream names (and the accepted argument shape) into the bundle, and do not
  count server-side validation failures against the tool budget.
* `/api/instances` degrades the synthesised instance id when the MCP probe fails
  (`CherishCai-96916` → `CherishCai`) and `matchQuality` moves `exact` → `host`, so identity
  quality changes with liveness. Instance id remains host-and-pid derived until doggy-dns
  publishes a stable `serviceInstanceId`.
* Case 4's stub run showed the agent's HTTP call to the OpenAI-compatible endpoint carrying
  `stream: null` (non-streaming request body) while the service consumes an event stream —
  harmless here, worth knowing when tuning per-turn deadlines.

## Runbook feedback

Cases 2, 3 and 7 could not be produced as the runbook wrote them, and they are three of
the eight denominator slots, so this round could not reach 80% even with perfect model
behaviour. What was applied afterwards — including why the case-3 suggestion below was
*not* taken — is in [Applied resolutions](#applied-resolutions-to-the-feedback-above):

* Case 3 needs a policy/ACL refusal path that does not exist in the resolver. Either drop the
  case or restate it as "upstream answered REFUSED quickly" (which is what the run observed).
* Case 2 assumes a refusal is distinguishable in the logs. doggy-dns emits one WARN
  message — `upstream forward lookup failed`, recording `response_code` and `error` — for
  every non-NOERROR / non-NXDOMAIN upstream error (`crates/doggy-dns-plugin/src/builtin/forward.rs`),
  so a timeout and a refusal share the message and the only fields that tell them apart are
  the recorded RCODE and error text; the observable distinction is the RCODE, not the
  message. That is also why the closed-port route fails to produce a refusal at all: over
  UDP it reaches this branch as a timeout, with `response_code=ServFail`.
* Case 7 needs a >200-row trace, which a read-only harness cannot build. Either mark it
  fixture-only (unit test over synthetic rows, which Task 3 already covers) or state that it
  requires a staging instance with debug logging enabled.

## Applied resolutions to the feedback above

* Case 2 was **restated rather than dropped**, and the suggestion to restate case
  3 as "upstream answered REFUSED quickly" was **not** taken: this run's own
  exploratory attempt proves that an upstream answering REFUSED is classified
  `upstream_refused` with valid citations, so folding it into case 3 would test
  case 2 twice and let a `policy_refused` claim go uncited. Case 2 is now
  "Upstream refused the query", reproduced by a REFUSED-answering upstream and
  checked on the RCODE; case 3 stays `policy_refused` and is marked fixture-only
  until doggy-dns ships a policy/ACL plugin.
* Case 7 is marked fixture-only with a pointer to the `select_logs_*` tests in
  `tests/evidence_test.rs`, which already pin the 200-row cap, the
  failure/WARN/ERROR priority, the first/last INFO and neighbour retention, and
  the `total`/`omitted`/`truncated` metadata.
* Metrics now states the consequence: the reachable ceiling against a real
  instance is 6 of 8, so a round reports both the 8-denominator score and the
  reachable-denominator score instead of letting a harness gap read as a model
  failure.

## Aftermath — 2026-11-02 re-check

Both blockers were fixed in code, not by relaxing the runbook:

* **B1** — PR #6 (`fix/evidence-fingerprint-stable`) excludes `instance.uptimeSeconds` and
  `instance.runtime` from the evidence fingerprint, and corrects the tests and
  spec sentence that required them to change it.
* **B2** — PR #7 (`fix/analysis-budget-alignment`) renders the enforced budget,
  unproductive allowance, deadline, result cap, configured stream names and
  declared tool list into the prompt (`failure-analyst/v2`), separates
  evidence-bearing drill-downs from attempts that return nothing (their own
  allowance of 4, no citable ID, disclosed as a gap), and raises the default
  `CONSOLE_ANALYSIS_TIMEOUT_SECONDS` from 60 to 180.

Re-run of case 1 on that build with every default in force (budget 6, allowance
4, 180 s, 8192 bytes): a fresh upstream-timeout trace returned `succeeded` /
result `ok` / `upstream_timeout` / **high** in **120 s** over 5 audited drill-down
calls, 4 of which returned evidence. The fifth was rejected upstream because the
model invented a field for `openobserve_trace_dag`; under the new accounting it
spent no evidence budget and minted no citable ID, and the submission disclosed
it as a gap. No call in the audit guessed a stream name — the model wrote
`FROM doggy_dns` and passed `stream_name: doggy_dns`, against 2-4 "stream not
found"/argument-shape rejections in this round.

Two caveats worth recording, both now stated in the runbook and README:

* 180 s is not headroom at the top of the budget. A separate attempt on the same
  build used all 6 evidence calls (525 ms of tool time in total) and was cut at
  the 180 s wall on the model turn after its budget ran out, storing
  `unavailable`/`low`. The measured `~(calls + 1) × 30 s` says a full-budget
  attempt wants ~210 s, so the defaults knowingly degrade the worst case; whether
  to raise the deadline or lower the budget is an open maintainer decision.
* The first 2026-11-02 attempt appeared to execute **7** drill-downs under a
  6-call budget. The cause was a stale process, not the gate: the running console
  predated the accounting fix. After a rebuild the same script executed exactly
  6. Anyone reading a `toolCallCount` above the budget should check the binary's
  build time before doubting the cap — and note the audit ceiling is
  `budget + 4`, since audits include failed attempts.

The live `tools/list` captured on 2026-11-02 also corrected one classification:
`openobserve_latest_traces` requires `stream_name` and was missing from the
prompt's stream guidance entirely.

## Artefacts

Harness, per-case snapshots and tool audits live in `/tmp/doggy-live/` and
`/tmp/doggy-live/eval/` (deliberately outside both repositories). Investigation rows are in
`data/console.db` (`failure_investigations`), 17 rows: 14 `qwen3.8-flash` attempts, 3
`stub-nx` attempts.
