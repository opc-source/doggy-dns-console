# Absolute Time Range & Failure List (A1 + A2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the failure investigation chain (list → evidence → analysis) be addressed by an absolute `from`/`to` millisecond bracket up to 7 days, filter it by instance host, and surface it on a new `/failures` page that owns the drawer URL.

**Architecture:** One `Range` bracket replaces `FailureFilter.window`; a single `Range::resolve` precedence rule serves both the HTTP query and the copilot tools. OpenObserve queries keep driving their existing `_timestamp` predicates from the bracket, and gain a case-insensitive `lower(host_name) = lower(?)` equality clause. The evidence fingerprint stops recording the requested range (logs/spans are matched by trace id and metrics are trace-centred), so one analysis is reusable across brackets. The frontend adds a `/failures` list route with the drawer mounted as its child, so `ui.drawerTraceId` and every silent clamp disappears.

**Tech Stack:** Rust 2024 (axum 0.8, tokio, async_trait, tracing, sqlx sqlite, serde, sha2, schemars, adk-rust), Vue 3 `<script setup>` + TypeScript + Pinia + vue-router + Tailwind v4, vitest, Playwright.

**Spec:** [docs/superpowers/specs/2026-11-03-absolute-range-failure-list-design.md](docs/superpowers/specs/2026-11-03-absolute-range-failure-list-design.md)

## Global Constraints

- Time units: internal epoch is **milliseconds**; OpenObserve `_timestamp` predicates are **microseconds** (`ms * 1_000`); PromQL is **seconds**. Never mix.
- Investigation-chain ceiling: `MIN_RANGE_MINUTES = 5`, `MAX_RANGE_MINUTES = 7 * 1440` (10080). Fixed constant — no new env var.
- Live-class ceiling (overview + instances) stays `Window::new` clamp to 5–1440 minutes. Do not widen it.
- Backend rejects exactly three things (spec D4): `start >= end`, span `< 5` minutes, span `> 7` days, plus `to > now + 60_000`. Anything else is accepted; the frontend must not clamp.
- `Range::resolve` precedence (D1): `from`/`to` win when both are present; otherwise `windowMinutes` (default 60) expands to `[to - n, to]` with `to` defaulting to `now`.
- The console is strictly read-only: no config writes, no DNS mutations, no alert management.
- SQLite stores agent sessions and investigation metadata only — never raw telemetry logs or spans.
- `.env` stays untracked; API keys and addresses come only from environment variables; no secrets in command lines, commit messages, or PR descriptions.
- Every commit that contains AI-assisted work ends with `Co-authored-by: Qoder <noreply@qoder.com>`.
- The user merges PRs. Never merge or push without an explicit instruction.
- `cargo fmt --check`, `cargo clippy --all-targets -- --D warnings`, `cargo test` must be clean after every backend task; `bun run type-check`, `bun run test:unit -- --run`, `bun run build` after every frontend task.
- Default `limit` stays 50 for `/api/failures`, 1 for the detail path, clamped to `1..=200` (spec §8: the 200 cap and the absence of `offset` are known limits, not bugs to fix here).
- `PROMPT_VERSION` stays `failure-analyst/v2` (D12) — no bump.

## Deviations and interpretations from the spec

Record these once; tasks reference them instead of repeating the argument.

1. **Spec §6.1 asks for two helpers, `investigation_filter` and `live_filter`.** Only `investigation_filter` is implemented. `live_filter` would have no caller — overview and `instance_detail` build `Window::new(...)` directly and `clippy -D warnings` rejects an unused `pub(crate)` helper. The live class keeps its existing relative-window construction verbatim.
2. **Spec D9 says copilot tools "share `FailureFilter`"; read literally that would give `get_overview` a `from`/`to`.** D2 pins the live class to relative 5–1440 minutes, so `get_overview` is left unchanged and only `list_failures` / `get_failure_detail` gain the absolute bracket. This is the narrower reading and the one that keeps the two ceilings honest.
3. **The clock goes into `ConsoleService`, not `ApiState`.** `ApiState` is a plain data bag; `ConsoleService` already owns the datasource `Arc`s, and the demo path anchors `FixtureSource` on the same clock (`src/server.rs:69`). Sharing one `Arc<dyn Clock>` between the service and its fixture source is what keeps HTTP unit tests (`FixedClock`) and demo mode (`SystemClock`) self-consistent once the fixture really filters by time.
4. **`Range` gets `#[derive(Default)]`, and `FailureFilter.range` gets `#[serde(default)]`.** A hand-written `impl Default for Range` would trip `clippy::derivable_impls`. `Default` therefore means `0..0`, which is inert because the post-D11 `EvidenceFilter` echo carries no range at all. This combination is what makes the D14 one-time stale of stored `filter_json` safe instead of a 500 at `src/analysis/store.rs:509`.
5. **Spec §4 has the failure-list URL carry `from`/`to` only for absolute drafts.** Relative drafts round-trip as `windowMinutes`, because the browser clock and the server clock can differ by seconds; sending a server-derived bracket would make the URL drift on every refresh.
6. **The new page's rows reuse `FailureRow.vue` with a `dense` prop** rather than duplicating markup, and the overview card keeps its own 5-row slice. Spec §5 describes the list page as owning the full-field rows; the overview card is deliberately left short.

## File Structure

**Backend — contract**
- `src/domain.rs` — add `MIN_RANGE_MINUTES`, `MAX_RANGE_MINUTES`, `FUTURE_TOLERANCE_MS`, `DEFAULT_WINDOW_MINUTES`, `Range` (`from_window` / `resolve` / `checked`); replace `FailureFilter.window` with `range` and add `instance`; add `FailureFilter::new(range)` while keeping `Default` (it delegates to `new(Range::default())` so the 11 `FailureFilter::default()` test sites survive untouched).
- `src/api/failures.rs` — `FailureQuery` gains `from`/`to`/`instance`; `filter_from(query, default_limit, now_ms)` (Task 2) → `investigation_filter(query, default_limit, now_ms) -> Result<FailureFilter, DataError>` (Task 3).
- `src/api/evidence.rs`, `src/api/analysis.rs` — call the shared resolver; `StartQuery` gains `from`/`to`/`instance`.
- `src/api/state.rs` — unchanged (see deviation 3).
- `src/service.rs` — owns `Arc<dyn Clock>`, exposes `now_ms()`; `instance_detail` counts with a `Range::from_window` bracket.
- `src/datasource/openobserve/telemetry.rs` — `failures` / `failure_detail` read `filter.range`; `overview` / `seen_instances` keep `range(window)`.
- `src/datasource/openobserve/failures.rs` — `extra_filters` adds the host equality clause.
- `src/evidence/model.rs` — `EvidenceFilter { domain, response_code, instance }`.
- `src/evidence/assemble.rs` — unchanged behaviour; the fingerprint naturally stops seeing the range.
- `src/analysis/prompt.rs` — the `filter:` legend line describes the new shape.
- `src/analysis/store.rs` — unchanged code, pinned by a serde test.
- `src/agent/tools.rs` — `FailureArgs`/`TraceArgs` gain `from`/`to`/`instance`; both failure tool descriptions state the real limits.
- `src/datasource/fixture.rs` — real time/instance filtering plus a spread of ages.
- `src/server.rs` — pass `clock.clone()` to both `ConsoleService::new` calls.

**Frontend**
- `src/api/types.ts` — mirror `EvidenceFilter`; add `InvestigationFilter`.
- `src/lib/failureFilter.ts` (new) — `FilterDraft`, `parseFilterQuery`, `relativeDraft`, `isAbsolute`, `resolveBracket`, `validateBracket`, `toRequestQuery`, `toRouteQuery`, `toInvestigationFilter`, `datetimeToMs`, `msToDatetime`, `rangeLabel`.
- `src/composables/useFailureList.ts` — take a `Ref<FilterDraft>`; validation guard, no interval for absolute brackets, `uncovered` count.
- `src/composables/useInvestigation.ts` — take a `Ref<InvestigationFilter>` instead of `windowMinutes`.
- `src/components/FailureRow.vue` (new), `src/components/FailureFilterBar.vue` (new), `src/views/FailuresView.vue` (new).
- `src/components/FailureList.vue`, `src/components/FailureDrawer.vue`, `src/views/OverviewView.vue`, `src/views/FailureView.vue`, `src/stores/ui.ts`, `src/router/index.ts`, `src/components/SideRail.vue`, `src/components/AiPanel.vue`.
- Delete `src/composables/useFailures.ts` and `src/composables/usePolling.ts` (no importers).

**Docs**
- `README.md` — `from`/`to`, the two ceilings, intended cross-range reuse.

---

### Task 1: `Range` value object and its rules

**Files:**
- Modify: `src/domain.rs` (add constants + `Range` after `Window`, i.e. after `src/domain.rs:22`)
- Test: `tests/domain_test.rs`

**Interfaces:**
- Consumes: nothing new; `Window` already exists at `src/domain.rs:11-22`.
- Produces: `MIN_RANGE_MINUTES: u32 = 5`, `MAX_RANGE_MINUTES: u32 = 7 * 1440`, `FUTURE_TOLERANCE_MS: i64 = 60_000`, `DEFAULT_WINDOW_MINUTES: u32 = 60`, `Range { start_ms: i64, end_ms: i64 }` with `Range::from_window(Window, i64) -> Range`, `Range::resolve(Option<i64>, Option<i64>, Option<u32>, i64) -> Result<Range, String>`, `Range::checked(i64, i64, i64) -> Result<Range, String>`. Task 2 puts `Range` into `FailureFilter`; Task 3 uses `resolve` in the HTTP layer; Task 6 uses it in the copilot tools.

- [ ] **Step 1: Write the failing tests**

Append to `tests/domain_test.rs` (the file already imports the domain prelude; add `Range` and the constants to that `use` list):

```rust
#[test]
fn range_from_window_expands_backwards_from_now() {
    let range = Range::from_window(Window::new(60), 1_761_436_800_000);
    assert_eq!(range.end_ms, 1_761_436_800_000);
    assert_eq!(range.start_ms, 1_761_436_800_000 - 60 * 60_000);
}

#[test]
fn range_resolve_prefers_explicit_from_and_to() {
    let range = Range::resolve(
        Some(1_000),
        Some(2_000),
        Some(60),
        9_000_000_000,
    )
    .unwrap();
    assert_eq!((range.start_ms, range.end_ms), (1_000, 2_000));
}

#[test]
fn range_resolve_falls_back_to_window_minutes_anchored_at_to() {
    // `to` is given, `from` is not: the window is measured backwards from `to`.
    let range = Range::resolve(None, Some(600_000), Some(10), 9_000_000_000).unwrap();
    assert_eq!((range.start_ms, range.end_ms), (0, 600_000));

    // Neither bound is given: `to` defaults to now and the window to 60 minutes.
    let range = Range::resolve(None, None, None, 1_761_436_800_000).unwrap();
    assert_eq!(range.end_ms, 1_761_436_800_000);
    assert_eq!(range.start_ms, 1_761_436_800_000 - 60 * 60_000);
}

#[test]
fn range_checked_rejects_the_four_documented_rules() {
    let now = 1_761_436_800_000;
    let ten_minutes = 10 * 60_000;

    // start >= end
    assert!(Range::checked(now, now, now).is_err());
    assert!(Range::checked(now, now - ten_minutes, now).is_err());

    // span shorter than the 5-minute floor
    assert!(Range::checked(now - 4 * 60_000, now, now).is_err());
    assert!(Range::checked(now - 5 * 60_000, now, now).is_ok());

    // span longer than the 7-day ceiling
    assert!(Range::checked(now - 7 * 1440 * 60_000 - 1, now, now).is_err());
    assert!(Range::checked(now - 7 * 1440 * 60_000, now, now).is_ok());

    // `to` beyond the future tolerance
    assert!(Range::checked(now - ten_minutes, now + 60_001, now).is_err());
    assert!(Range::checked(now - ten_minutes, now + 60_000, now).is_ok());
}

#[test]
fn range_resolve_reports_the_rejection_reason() {
    let now = 1_761_436_800_000;
    let error = Range::resolve(Some(now), Some(now), None, now).unwrap_err();
    assert!(error.contains("start"), "unexpected error: {error}");

    let error = Range::resolve(Some(now - 60 * 60_000), Some(now + 120_000), None, now).unwrap_err();
    assert!(error.contains("future"), "unexpected error: {error}");
}
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test domain_test range_ 2>&1 | tail -20`
Expected: compile error `cannot find ... Range` (E0432 / E0433).

- [ ] **Step 3: Write `Range`**

Insert into `src/domain.rs` immediately after `impl Default for Window` (`src/domain.rs:18-22`):

```rust
pub const MIN_RANGE_MINUTES: u32 = 5;
/// The investigation chain may look back a full week; the live class stays at
/// 1440 minutes and keeps clamping through [`Window::new`].
pub const MAX_RANGE_MINUTES: u32 = 7 * 1440;
/// Clock skew between the console and the browser, in milliseconds.
pub const FUTURE_TOLERANCE_MS: i64 = 60_000;
pub const DEFAULT_WINDOW_MINUTES: u32 = 60;

/// An absolute `[start, end]` bracket in epoch milliseconds.
///
/// `Default` is the deliberately inert `0..0`: legacy `filter_json` blobs stored
/// before the range was introduced decode into it, and nothing reads the range
/// off the evidence echo any more (see the fingerprint decoupling).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Range {
    pub fn new(start_ms: i64, end_ms: i64) -> Self {
        Self { start_ms, end_ms }
    }

    /// Expand a relative window into `[now - minutes, now]`.
    pub fn from_window(window: Window, now_ms: i64) -> Self {
        let span = i64::from(window.minutes) * 60_000;
        Self::new(now_ms - span, now_ms)
    }

    /// Apply the precedence rule: an explicit `from`/`to` pair wins, otherwise
    /// `windowMinutes` (default 60) is measured backwards from `to` (default
    /// `now`). A lone `from` or lone `to` still participates, which is why the
    /// fallback anchors on the resolved `end`.
    pub fn resolve(
        from: Option<i64>,
        to: Option<i64>,
        window_minutes: Option<u32>,
        now_ms: i64,
    ) -> Result<Self, String> {
        let end_ms = to.unwrap_or(now_ms);
        let minutes = i64::from(window_minutes.unwrap_or(DEFAULT_WINDOW_MINUTES));
        let start_ms = from.unwrap_or(end_ms - minutes * 60_000);
        Self::checked(start_ms, end_ms, now_ms)
    }

    /// The three rejection rules plus the future tolerance, and nothing else.
    pub fn checked(start_ms: i64, end_ms: i64, now_ms: i64) -> Result<Self, String> {
        if start_ms >= end_ms {
            return Err("range start must be before range end".to_owned());
        }
        if end_ms > now_ms + FUTURE_TOLERANCE_MS {
            return Err("range end is too far in the future".to_owned());
        }
        let span_ms = end_ms - start_ms;
        if span_ms < i64::from(MIN_RANGE_MINUTES) * 60_000 {
            return Err(format!("range is shorter than the {MIN_RANGE_MINUTES}-minute floor"));
        }
        if span_ms > i64::from(MAX_RANGE_MINUTES) * 60_000 {
            return Err(format!(
                "range is longer than the {MAX_RANGE_MINUTES}-minute ceiling"
            ));
        }
        Ok(Self::new(start_ms, end_ms))
    }
}
```

- [ ] **Step 4: Run the tests to confirm they pass**

Run: `cargo test --test domain_test 2>&1 | tail -20`
Expected: PASS, including the pre-existing `window_new_clamps_to_the_supported_band` test at `tests/domain_test.rs:19-22`.

- [ ] **Step 5: Check the whole crate still compiles and lint clean**

Run: `cargo fmt --check && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: no output beyond `Finished`. If `clippy` reports the range checks as `collapsible_if`, keep the early returns as written — each has a distinct message.

- [ ] **Step 6: Commit**

```bash
git add src/domain.rs tests/domain_test.rs
git commit -m "$(cat <<'EOF'
feat: add an absolute Range bracket for the investigation chain
EOF
)"
```

---

### Task 2: `FailureFilter` carries the bracket; `ConsoleService` owns the clock

This task is a behaviour-preserving migration: every existing test keeps passing with the same numbers, and no new rule is reachable from HTTP yet (that is Task 3). It is deliberately one commit because the contract is unusable until all construction sites move together.

**Files:**
- Modify: `src/domain.rs:55-62` (`FailureFilter`) and `src/domain.rs:214-223` (`impl Default for FailureFilter`)
- Modify: `src/service.rs` (add the clock, `now_ms()`, `evidence()`, `instance_detail`)
- Modify: `src/api/failures.rs:38-46` (`filter_from` gains `now_ms`)
- Modify: `src/api/analysis.rs:161-175` (`start()` passes `now_ms`)
- Modify: `src/agent/tools.rs:64-69,78-83` (resolve a `Range` from `window_minutes`)
- Modify: `src/datasource/openobserve/telemetry.rs:83,100` (read `filter.range`)
- Modify: `src/evidence/model.rs:42-59` (interim bridge: `window_minutes` derived from the span; Task 5 deletes it)
- Modify: `src/server.rs:82,111` (`ConsoleService::new` gains the clock)
- Test: `tests/domain_test.rs`, plus the mechanical literals in `tests/failures_test.rs:40,148`, `tests/evidence_test.rs:1415`, `tests/api_analysis_test.rs:58`, `tests/analysis_test.rs:1396`, `tests/fixture_test.rs:29,39`, and the nine other `ConsoleService::new` sites.

**Interfaces:**
- Consumes: `Range`, `Range::from_window`, `Range::checked`, `DEFAULT_WINDOW_MINUTES` (Task 1).
- Produces: `FailureFilter { range: Range, domain: Option<String>, response_code: Option<String>, instance: Option<String>, limit: usize }`; `FailureFilter::new(range: Range) -> FailureFilter` (limit 50, no clamping); `ConsoleService::new(Arc<dyn TelemetrySource>, Arc<dyn InstanceSource>, Arc<dyn Clock>)`; `ConsoleService::now_ms(&self) -> i64`; `filter_from(query: FailureQuery, default_limit: usize, now_ms: i64) -> FailureFilter` (Task 3 turns this into `investigation_filter`).

- [ ] **Step 1: Write the failing decode pin (the D14 hazard)**

The investigation store decodes the `filter_json` column it wrote months ago (`src/analysis/store.rs:509`). Old rows contain `{"window":{"minutes":60},...,"limit":50}` and no `range`. Append to `tests/domain_test.rs`:

```rust
#[test]
fn legacy_failure_filter_json_still_decodes() {
    // What src/analysis/store.rs wrote before the absolute range existed.
    // `SqliteInvestigationStore::decode_record` runs this same `from_str`, so a
    // failure here would be a 500 on every historical row.
    let legacy = r#"{"window":{"minutes":60},"domain":null,"responseCode":null,"limit":50}"#;
    let filter: FailureFilter = serde_json::from_str(legacy).unwrap();

    assert_eq!(filter.limit, 50);
    assert_eq!(filter.domain, None);
    assert_eq!(filter.response_code, None);
    assert_eq!(filter.instance, None);
    // The unknown `window` key is ignored and the missing `range` decodes to the
    // inert 0..0 bracket; nothing reads it off the evidence echo after the
    // fingerprint decoupling.
    assert_eq!(filter.range, Range::default());
    assert_eq!(filter.range.start_ms, 0);
    assert_eq!(filter.range.end_ms, 0);
}

#[test]
fn failure_filter_new_defaults_the_limit() {
    let filter = FailureFilter::new(Range::new(1_000, 2_000));
    assert_eq!(filter.range, Range::new(1_000, 2_000));
    assert_eq!(filter.limit, 50);
    assert_eq!(filter.domain, None);
    assert_eq!(filter.response_code, None);
    assert_eq!(filter.instance, None);

    // Default stays available for tests and is defined as `new(0..0)`; the live
    // limit of 50 keeps it out of `#[derive(Default)]`'s reach.
    assert_eq!(FailureFilter::default(), FailureFilter::new(Range::default()));
}
```

Also add `FailureFilter` to the `use doggy_dns_console::domain::{...}` list at the top of `tests/domain_test.rs`.

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test --test domain_test legacy 2>&1 | tail -20`
Expected: compile error — `FailureFilter` has no field `range`, no `instance`, no `new`.

- [ ] **Step 3: Reshape `FailureFilter`**

Replace `src/domain.rs:55-62` with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailureFilter {
    /// The absolute bracket the investigation chain queries. `#[serde(default)]`
    /// keeps rows written before this field existed decodable.
    #[serde(default)]
    pub range: Range,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    /// Case-insensitive equality against the telemetry `host_name` column.
    pub instance: Option<String>,
    pub limit: usize,
}
```

Replace `impl Default for FailureFilter` (`src/domain.rs:214-223`) with:

```rust
impl FailureFilter {
    pub fn new(range: Range) -> Self {
        Self {
            range,
            domain: None,
            response_code: None,
            instance: None,
            limit: 50,
        }
    }
}

impl Default for FailureFilter {
    fn default() -> Self {
        Self::new(Range::default())
    }
}
```

- [ ] **Step 4: Give `ConsoleService` the clock**

Read `src/service.rs` first; it currently constructs `SystemClock` inline inside `evidence()` (`src/service.rs:61`). Change the struct and constructor:

```rust
use crate::datasource::{Clock, InstanceSource, TelemetrySource};

pub struct ConsoleService {
    telemetry: Arc<dyn TelemetrySource>,
    instances: Arc<dyn InstanceSource>,
    clock: Arc<dyn Clock>,
}

impl ConsoleService {
    pub fn new(
        telemetry: Arc<dyn TelemetrySource>,
        instances: Arc<dyn InstanceSource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            telemetry,
            instances,
            clock,
        }
    }

    /// The single source of truth for "now" behind every resolved bracket, so
    /// the service and its datasource agree even in demo mode, where the fixture
    /// is anchored on the same clock.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }
```

Then in `evidence()` replace the inline `&SystemClock` argument with `&self.clock`, and in `instance_detail` (`src/service.rs:72-75`) build the bracket explicitly:

```rust
let filter = FailureFilter::new(Range::from_window(Window::new(60), self.now_ms()));
```

Update the `use crate::domain::{...}` line in `src/service.rs:5` to import `Range` as well.

- [ ] **Step 5: Move the query layer to `now_ms`**

`src/api/failures.rs:38-46` becomes (the `Window` import drops out of `src/api/failures.rs:7`):

```rust
pub(crate) fn filter_from(query: FailureQuery, default_limit: usize, now_ms: i64) -> FailureFilter {
    FailureFilter {
        range: Range::from_window(Window::new(query.window_minutes.unwrap_or(60)), now_ms),
        domain: query.domain,
        response_code: query.response_code,
        instance: None,
        limit: query.limit.unwrap_or(default_limit).clamp(1, 200),
    }
}
```

For now import `Window` alongside `Range` (`use crate::domain::{FailureDetail, FailureFilter, FailureGroup, Range, Window};`) — Task 3 removes `Window` again. Both handlers call it with the service clock, e.g.:

```rust
let filter = filter_from(query, 50, state.service.now_ms());
```

`src/api/analysis.rs:161-175` (`start()`) rebuilds a `FailureQuery` from `StartQuery`; make it carry through the same three fields it already knows and resolve against `state.service.now_ms()`, so the only new statement is the `now_ms` argument:

```rust
let filter = crate::api::failures::filter_from(
    FailureQuery {
        window_minutes: query.window_minutes,
        domain: query.domain,
        response_code: query.response_code,
        limit: None,
    },
    1,
    state.service.now_ms(),
);
```

- [ ] **Step 6: Move the datasource and the copilot tools**

`src/datasource/openobserve/telemetry.rs:83` and `:100` stop asking for a window:

```rust
let (start, end) = (filter.range.start_ms, filter.range.end_ms);
```

Leave `fn range(&self, window: Window)` (`src/datasource/openobserve/telemetry.rs:34-37`) in place — `overview` (`:43`) and `seen_instances` still use it (D2).

`src/agent/tools.rs:64-69` and `:78-83` build the bracket from the tool arguments; `Window` stays imported because `get_overview` (`:55`) still uses it. Keep each site's own default limit (`list_failures` → `args.limit.unwrap_or(50).clamp(1, 200)`, `get_failure_detail` → `1`):

```rust
let now_ms = service.now_ms();
let range = Range::checked(
    now_ms - i64::from(args.window_minutes.unwrap_or(60)) * 60_000,
    now_ms,
    now_ms,
)
.unwrap_or_else(|_| Range::from_window(Window::new(DEFAULT_WINDOW_MINUTES), now_ms));
let filter = FailureFilter {
    range,
    domain: args.domain,
    response_code: args.response_code,
    instance: None,
    limit: args.limit.unwrap_or(50).clamp(1, 200),
};
```

In `get_failure_detail` the same two lines apply minus `limit` (it stays `1`) and minus the `args.domain`/`response_code` rename. Task 6 replaces this `unwrap_or_else` bridge with a real `Range::resolve` plus error propagation; the bridge exists only so Task 2's commit is behaviour-preserving.

- [ ] **Step 7: Keep the evidence echo compiling (interim only)**

`src/evidence/model.rs:51-58` becomes a span-derived bridge; the field itself disappears in Task 5, and `From<&FailureFilter>` keeps `window_minutes` non-zero for the existing 60-minute assertions:

```rust
impl From<&FailureFilter> for EvidenceFilter {
    fn from(filter: &FailureFilter) -> Self {
        let span_minutes = ((filter.range.end_ms - filter.range.start_ms) / 60_000).max(1);
        Self {
            window_minutes: span_minutes as u32,
            domain: filter.domain.clone(),
            response_code: filter.response_code.clone(),
        }
    }
}
```

- [ ] **Step 8: Migrate the literals and the constructor sites**

`src/server.rs:82` and `:111` become `ConsoleService::new(telemetry, instances, clock.clone())` (the `clock` binding already exists at `src/server.rs:65`). In the demo branch the `Arc<dyn Clock>` is moved into `FixtureSource::new(clock.clone())` first, so keep the clone.

Rewrite the nine test `ConsoleService::new(...)` sites to pass a clock — `Arc::new(FixedClock(NOW_MS))` where the file already has a fixed instant, `Arc::new(SystemClock)` otherwise:
`tests/api_test.rs:85`, `tests/api_evidence_test.rs:60`, `tests/api_analysis_test.rs:128`, `tests/agent_test.rs:50`, `tests/agui_test.rs:65`, `tests/fixture_test.rs:14`, `tests/tools_test.rs:12`, `tests/evidence_test.rs:2293`.

Replace the `window:` field in the six struct literals with a bracket that yields the same 60-minute span they had before:

| site | replacement |
| --- | --- |
| `tests/failures_test.rs:40-45` (`window_filter`) | `range: Range::new(NOW_MS - 60 * 60_000, NOW_MS), instance: None,` |
| `tests/failures_test.rs:148-155` (SQL text test) | `range: Range::new(1, 2), instance: None,` — the query takes explicit `start_ms`/`end_ms` arguments, so the bracket is unused |
| `tests/evidence_test.rs:1414-1421` (`assemble_filter`) | `range: Range::new(1, 4_000_000_000_000), instance: None,` — deliberately the widest bracket, not 60 minutes: `FakeTelemetry` ignores the range, and `console_service_evidence_delegates_to_assembly_with_fixture_sources` (`:2290-2326`) runs the **real** fixture whose clock is `FixedClock(1_761_436_800_000)` while `GENERATED_AT` in that file is `1_793_443_200_000`. Anchoring on `GENERATED_AT` would put every fixture row outside the bracket and turn that test into a 404 once Task 7 makes the fixture filter for real. |
| `tests/api_analysis_test.rs:56-62` (`filter`) | `range: Range::new(NOW_MS - 60 * 60_000, NOW_MS), instance: None,` |
| `tests/analysis_test.rs:1395-1402` (`sample_filter`) | `range: Range::new(GENERATED - 30 * 60_000, GENERATED), instance: None,` where `GENERATED` is the `1_761_436_800_000` literal the file already uses; keep asserting `Some(VerdictCategory::UpstreamTimeout)` etc. |
| `tests/fixture_test.rs:29-33,39-43` | `range: Range::new(0, 1_761_436_800_000 + 60 * 60_000), instance: None,` — the widest bracket so Task 7's real filtering cannot hide a row; Task 7 rewrites these to prove the filter works |

Add `Range` (and `FixedClock`/`SystemClock`/`Clock` where missing) to each file's `use` list. `tests/analysis_test.rs:27,1956,1996,2024,2062,2158,2207,2241,2278,2312,2339` keep using `FailureFilter::default()` — that is why `Default` stays.

- [ ] **Step 9: Run the full backend suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. Every assertion still sees the numbers it did before, because each migrated literal spans the same 60 (or 30) minutes.

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean. If clippy flags `field_reassign_with_default`, use struct literals rather than `FailureFilter::default()` then assignment.

- [ ] **Step 10: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
refactor: put the absolute bracket on FailureFilter and share one clock

Behaviour preserving: every window is expanded to [now-n, now] at the edge, so
queries and the evidence echo keep their previous values. Legacy filter_json
still decodes thanks to #[serde(default)] on the new range field.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 3: `from`/`to`/`instance` become reachable from HTTP

One shared resolver serves all four investigation endpoints, so the precedence rule (D1) and the rejection list (D4) exist once.

**Files:**
- Modify: `src/api/failures.rs` (`FailureQuery`, `filter_from` → `investigation_filter`, both handlers, drop the `Window` import)
- Modify: `src/api/evidence.rs:5,20` and its doc comment (`:11-14`)
- Modify: `src/api/analysis.rs:28,41,161-175` (`StartQuery` gains `from`/`to`/`instance`)
- Test: `tests/api_test.rs`, `tests/api_evidence_test.rs`

**Interfaces:**
- Consumes: `Range::resolve`, `Range::checked`, `FUTURE_TOLERANCE_MS`, `MIN_RANGE_MINUTES`, `MAX_RANGE_MINUTES`, `DEFAULT_WINDOW_MINUTES` (Task 1); `FailureFilter { range, instance, .. }` (Task 2); `DataError::Invalid` → 400 `invalid_request` (already mapped in `src/api/error.rs`).
- Produces: `FailureQuery { window_minutes, from, to, domain, response_code, instance, limit }`; `pub(crate) fn investigation_filter(query: FailureQuery, default_limit: usize, now_ms: i64) -> Result<FailureFilter, DataError>`; `StartQuery { window_minutes, from, to, domain, response_code, instance, force }`. Task 6 mirrors the same precedence in the copilot tools.

- [ ] **Step 1: Write the failing precedence and rejection tests**

Append to `tests/api_test.rs`. `RecordingTelemetry` currently records only `filter.limit` through an `Arc<AtomicUsize>` (`tests/api_test.rs:37-40`); widen it to the whole filter so the resolver is observable end to end — change the field type to `Arc<Mutex<Option<FailureFilter>>>` (keep `Debug` by storing an `Option`), name it `last_filter`, and update the two existing limit tests (`tests/api_test.rs:157-191`) to read it back:

```rust
#[tokio::test]
async fn failures_from_and_to_win_over_window_minutes() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?from=1761433200000&to=1761436800000&windowMinutes=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.range, Range::new(1_761_433_200_000, 1_761_436_800_000));
}

#[tokio::test]
async fn failures_window_minutes_expands_backwards_from_to() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?to=1761436800000&windowMinutes=120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.range, Range::new(1_761_429_600_000, 1_761_436_800_000));
}

#[tokio::test]
async fn failures_reject_the_four_bracket_rules_with_400() {
    let now_ms = 1_761_436_800_000;
    let cases = [
        // start >= end
        format!("/api/failures?from={now_ms}&to={now_ms}").leak() as &str,
        // below the 5-minute floor
        format!("/api/failures?from={}&to={}", now_ms - 4 * 60_000, now_ms).leak() as &str,
        // above the 7-day ceiling
        format!(
            "/api/failures?from={}&to={}",
            now_ms - 7 * 1440 * 60_000 - 1,
            now_ms
        )
        .leak() as &str,
        // `to` beyond the 60-second future tolerance
        format!("/api/failures?from={}&to={}", now_ms, now_ms + 60_001).leak() as &str,
        // a non-numeric bracket bound
        "/api/failures?from=abc&to=1".leak() as &str,
    ];

    for uri in cases {
        let (app, _recorder) = app_with_recorder().await;
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "expected 400 for {uri}"
        );
    }
}

#[tokio::test]
async fn failures_accept_the_seven_day_edge_and_the_future_tolerance() {
    let now_ms = 1_761_436_800_000;
    for uri in [
        format!(
            "/api/failures?from={}&to={}",
            now_ms - 7 * 1440 * 60_000,
            now_ms
        )
        .leak() as &str,
        format!("/api/failures?from={}&to={}", now_ms - 60 * 60_000, now_ms + 60_000).leak()
            as &str,
        // exactly the 5-minute floor
        format!("/api/failures?from={}&to={}", now_ms - 5 * 60_000, now_ms).leak() as &str,
    ] {
        let (app, _recorder) = app_with_recorder().await;
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "expected 200 for {uri}");
    }
}

#[tokio::test]
async fn failures_pass_the_instance_and_default_a_relative_hour() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures?instance=dns-node-2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.instance.as_deref(), Some("dns-node-2"));
    assert_eq!(
        filter.range.end_ms - filter.range.start_ms,
        60 * 60_000,
        "the default bracket is one hour"
    );
}
```

Add `use doggy_dns_console::domain::Range;` and `use std::sync::Mutex;` to `tests/api_test.rs`. The existing `app_with` helper (`tests/api_test.rs:82-94`) builds the router over a `FixedClock(1_761_436_800_000)`-anchored service once Task 3 Step 4 lands — write `app_with_recorder()` next to it so it returns `(Router, Arc<RecordingTelemetry>)` and passes that same clock to `ConsoleService::new`, which makes `now_ms` deterministic and equal to `1_761_436_800_000`. Keep `app_with` as a thin wrapper (`app_with_recorder().await.map(Into::into)`) so the untouched tests still compile.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test api_test 2>&1 | tail -30`
Expected: the new tests fail — `from`/`to`/`instance` are ignored (`FailureQuery` has no such fields, so the query string silently drops them) and no case returns 400.

- [ ] **Step 3: Extend the query structs**

`src/api/failures.rs:9-17` becomes:

```rust
#[derive(Debug, Deserialize)]
pub struct FailureQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    /// Absolute bracket start in epoch milliseconds. Wins over `windowMinutes`
    /// when `to` is present too.
    pub from: Option<i64>,
    /// Absolute bracket end in epoch milliseconds; also anchors `windowMinutes`
    /// when `from` is absent. Defaults to the server clock.
    pub to: Option<i64>,
    pub domain: Option<String>,
    #[serde(rename = "responseCode")]
    pub response_code: Option<String>,
    pub instance: Option<String>,
    pub limit: Option<usize>,
}
```

Every literal construction of `FailureQuery` must gain the three fields — `src/api/analysis.rs:167-173` is the only one in `src`, plus the test helpers touched below.

- [ ] **Step 4: Replace the builder with the resolver**

`src/api/failures.rs:38-45` becomes (and `Window` leaves the import list at `:7`):

```rust
/// Turn a query string into the filter the investigation chain runs with.
///
/// The rejection list is exactly four rules, and the caller's bracket is never
/// clamped: an out-of-range request is a client bug, not something to guess at.
pub(crate) fn investigation_filter(
    query: FailureQuery,
    default_limit: usize,
    now_ms: i64,
) -> Result<FailureFilter, DataError> {
    let range = Range::resolve(query.from, query.to, query.window_minutes, now_ms)
        .map_err(DataError::Invalid)?;
    Ok(FailureFilter {
        range,
        domain: query.domain,
        response_code: query.response_code,
        instance: query.instance,
        limit: query.limit.unwrap_or(default_limit).clamp(1, 200),
    })
}
```

Add `use crate::error::DataError;` and `use crate::domain::{FailureDetail, FailureFilter, FailureGroup, Range};`. Both handlers propagate:

```rust
let filter = investigation_filter(query, 50, state.service.now_ms())?;
```

`ApiError` already maps `DataError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request")`, so no error-type work is needed.

- [ ] **Step 5: Wire the evidence and analysis endpoints**

`src/api/evidence.rs:20` and `src/api/analysis.rs:167` call `investigation_filter(...)` with `state.service.now_ms()` and `?`; update the `use crate::api::failures::{FailureQuery, filter_from};` lines (`src/api/evidence.rs:5`, `src/api/analysis.rs:28`) to import `investigation_filter`. Rewrite the stale sentence in `src/api/evidence.rs:11-14` so it names the bracket rather than "the same window":

```rust
/// The bundle is assembled over the same bracket/domain/responseCode/instance
/// filter as the list and detail views it is opened from, so it cannot drift
/// from the entry the operator clicked. The bracket does not enter the
/// fingerprint: logs and spans are matched by trace id and the metric window is
/// centred on the trace, so one analysis is intentionally reusable across
/// overlapping brackets. The process identity is derived server-side from the
/// trace; no identity is accepted from the caller.
```

`StartQuery` (`src/api/analysis.rs:39-46`) gains `from`, `to`, `instance` with the same serde names, and `start()` copies them into the `FailureQuery` it rebuilds.

- [ ] **Step 6: Align the evidence-endpoint tests**

`tests/api_evidence_test.rs` builds a `FailureQuery`-driven app; its existing cases at `:161-169` (malformed query → 400) and `:171-179` (upstream failure → 502) must still pass. Add one case pinning the bracket through the evidence route:

```rust
#[tokio::test]
async fn evidence_rejects_a_bracket_over_the_seven_day_ceiling() {
    let now_ms = 1_761_436_800_000;
    let app = app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/api/failures/trace-1/evidence?from={}&to={}",
                    now_ms - 7 * 1440 * 60_000 - 1,
                    now_ms
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 7: Run the backend suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. `tests/api_evidence_test.rs:99` (`body["filter"]["windowMinutes"] == 60`) and `:156-157` still hold — that test asks for the default relative hour.

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean. If `clippy` objects to `.leak() as &str` in the tests, keep the `String`s in a `Vec<String>` and iterate over `&uri` instead; the assertion text matters, the leak does not.

- [ ] **Step 8: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
feat: accept from/to and instance on the investigation endpoints

The bracket resolver is shared by the list, detail, evidence and analysis
start routes, and rejects rather than clamps: start>=end, spans under 5 or
over 7 days, and a `to` more than 60s in the future all answer 400.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 4: Instance equality predicate and the "uncovered" count

D6 has two halves: `lower(host_name) = lower(?)` narrows the group query, and because rows without a `host_name` are silently dropped by that equality, the same bracket must report how many rows the filter cannot reach (spec §6.2). One lightweight count query pays for it.

**Files:**
- Modify: `src/datasource/openobserve/failures.rs:87-102` (`extra_filters` → `scope_filters` + host clause) and add `unlabeled_failures_query`
- Modify: `src/datasource/mod.rs:47-71` (`TelemetrySource::unlabeled_failures` with a default implementation)
- Modify: `src/datasource/openobserve/telemetry.rs` (implement the count, use `filter.range`)
- Modify: `src/service.rs` (pass-through `unlabeled_failures`)
- Modify: `src/api/failures.rs` (new `uncovered` handler + `UncoveredEvidence` DTO)
- Modify: `src/api/mod.rs:24-26` (register `/api/failures/uncovered`)
- Modify: `src/datasource/fixture.rs` (Task 7 makes this count real; here it returns `Ok(0)` via the default)
- Test: `tests/failures_test.rs`, `tests/api_test.rs`

**Interfaces:**
- Consumes: `FailureFilter.instance` (Task 2), `investigation_filter` (Task 3), `sql_escape` (already used by `extra_filters`).
- Produces: `fn scope_filters(filter: &FailureFilter) -> String`, `fn unlabeled_failures_query(stream: &str, filter: &FailureFilter, start_ms: i64, end_ms: i64) -> String`, `TelemetrySource::unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError>`, `ConsoleService::unlabeled_failures(filter: FailureFilter) -> Result<u64, DataError>`, `GET /api/failures/uncovered` → `{ "unlabeled": number }`. Task 9 consumes the endpoint from the list page.

- [ ] **Step 1: Write the failing SQL text tests**

Append to `tests/failures_test.rs` (it already calls `failure_groups_query("doggy_dns", &filter, 1_000, 2_000, 50)` at `:146-180`, and `window_filter(limit)` is the shared builder from Task 2):

```rust
#[test]
fn instance_filter_is_a_case_insensitive_equality() {
    let mut filter = window_filter(50);
    filter.instance = Some("DNS-Node-2".to_owned());

    let sql = failure_groups_query("doggy_dns", &filter, 1_000, 2_000, 50);
    assert!(
        sql.contains("lower(host_name) = lower('DNS-Node-2')"),
        "unexpected SQL: {sql}"
    );
    // The bracket is still microsecond `_timestamp` predicates, not the ms pair.
    assert!(sql.contains("_timestamp >= 1000000"), "unexpected SQL: {sql}");
    assert!(sql.contains("_timestamp <= 2000000"), "unexpected SQL: {sql}");
}

#[test]
fn no_instance_filter_adds_no_host_clause() {
    let filter = window_filter(50);
    let sql = failure_groups_query("doggy_dns", &filter, 1_000, 2_000, 50);
    assert!(!sql.contains("host_name"), "unexpected SQL: {sql}");
}

#[test]
fn unlabeled_count_query_keeps_the_scope_but_drops_the_host_clause() {
    let mut filter = window_filter(50);
    filter.instance = Some("dns-node-2".to_owned());
    filter.response_code = Some("REFUSED".to_owned());

    let sql = unlabeled_failures_query("doggy_dns", &filter, 1_000, 2_000);

    assert!(sql.contains("COUNT(*)"), "unexpected SQL: {sql}");
    assert!(
        sql.contains("(host_name IS NULL OR host_name = '')"),
        "unexpected SQL: {sql}"
    );
    // The responseCode stays in scope — the count answers "what this filter
    // could not reach", not "everything in the bracket".
    assert!(
        sql.contains("lower(response_code) = lower('REFUSED')"),
        "unexpected SQL: {sql}"
    );
    // …and the host equality must not appear, or the count would be zero by
    // construction.
    assert!(!sql.contains("lower(host_name)"), "unexpected SQL: {sql}");
    assert!(sql.contains("_timestamp >= 1000000"), "unexpected SQL: {sql}");
}

#[test]
fn instance_and_host_clause_escapes_quotes() {
    let mut filter = window_filter(50);
    filter.instance = Some("node'\" OR 1=1".to_owned());

    let sql = failure_groups_query("doggy_dns", &filter, 1_000, 2_000, 50);
    assert!(!sql.contains("OR 1=1"), "unescaped injection: {sql}");
}
```

Add `unlabeled_failures_query` to the `use doggy_dns_console::datasource::openobserve::failures::{...}` import list at the top of `tests/failures_test.rs` (match whatever path the existing `failure_groups_query` import uses).

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test failures_test 2>&1 | tail -20`
Expected: compile error — `unlabeled_failures_query` does not exist, and the equality assertions fail once it does but before the clause is added.

- [ ] **Step 3: Split the clause builder and add both queries**

Replace `extra_filters` (`src/datasource/openobserve/failures.rs:87-102`) with:

```rust
/// Everything except the host equality: the bracket's failure scope.
fn scope_filters(filter: &FailureFilter) -> String {
    let mut clauses = String::new();
    if let Some(code) = &filter.response_code {
        clauses.push_str(&format!(
            " AND lower(response_code) = lower('{}')",
            sql_escape(code)
        ));
    }
    if let Some(domain) = &filter.domain {
        clauses.push_str(&format!(
            " AND position(lower('{}') in lower(coalesce(query, query_name))) > 0",
            sql_escape(domain)
        ));
    }
    clauses
}

/// The instance narrowing is an equality, not a substring: the console derives
/// instance identity from hostname-ip / host-pid heuristics, and a fuzzy host
/// match would silently group other nodes' rows into the answer.
fn extra_filters(filter: &FailureFilter) -> String {
    let mut clauses = scope_filters(filter);
    if let Some(instance) = &filter.instance {
        clauses.push_str(&format!(
            " AND lower(host_name) = lower('{}')",
            sql_escape(instance)
        ));
    }
    clauses
}

/// How many failures in the same scope carry no `host_name` at all, and are
/// therefore unreachable by the instance equality above. Reported next to the
/// filtered list so an empty result is never mistaken for a healthy node.
pub fn unlabeled_failures_query(
    stream: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!(
        "SELECT COUNT(*) AS count FROM \"{stream}\" \
         WHERE _timestamp >= {} AND _timestamp <= {}{} \
         AND (host_name IS NULL OR host_name = '')",
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
        scope_filters(filter),
    )
}
```

- [ ] **Step 4: Add the capability to the trait and the OpenObserve source**

In `src/datasource/mod.rs`, add to `pub trait TelemetrySource` (after `failure_detail`, `src/datasource/mod.rs:50-60`):

```rust
/// Failures in the filter's bracket and domain/responseCode scope that carry no
/// host label, so the instance equality in the list query cannot reach them.
///
/// Sources that cannot count them keep the default of zero; the UI then hides
/// the notice instead of inventing a number.
async fn unlabeled_failures(&self, _filter: FailureFilter) -> Result<u64, DataError> {
    Ok(0)
}
```

In `src/datasource/openobserve/telemetry.rs`, implement it after `failure_detail`, reading the count out of the single row `search_logs` returns:

```rust
async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
    let (start, end) = (filter.range.start_ms, filter.range.end_ms);
    let sql = unlabeled_failures_query(&self.client.config().log_stream, &filter, start, end);
    let rows = self.client.search_logs(&sql, start, end, 1).await?;
    let unlabeled = rows
        .iter()
        .find_map(|row| row.get("count").and_then(serde_json::Value::as_i64))
        .unwrap_or_default();
    Ok(unlabeled.max(0) as u64)
}
```

Add `unlabeled_failures_query` to the `use super::failures::{...}` import in that file, and match whatever row type `normalize_failure_group_row` already consumes — if `search_logs` returns `&Value` rows, `row.get("count")` is already correct.

- [ ] **Step 5: Expose it through the service and one endpoint**

`src/service.rs` gains, next to `failures()`:

```rust
pub async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
    self.telemetry.unlabeled_failures(filter).await
}
```

`src/api/failures.rs` gains the DTO and handler:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncoveredEvidence {
    pub unlabeled: u64,
}

/// Counts failures in the bracket that have no host label, i.e. the rows the
/// `instance` filter on `GET /api/failures` structurally cannot show.
///
/// Registered before `/api/failures/{trace_id}` so the literal path is not
/// swallowed by the trace-id segment; `uncovered_is_not_matched_by_the_trace_route`
/// pins that.
pub async fn uncovered(
    State(state): State<ApiState>,
    Query(query): Query<FailureQuery>,
) -> Result<Json<UncoveredEvidence>, ApiError> {
    let filter = investigation_filter(query, 1, state.service.now_ms())?;
    Ok(Json(UncoveredEvidence {
        unlabeled: state.service.unlabeled_failures(filter).await?,
    }))
}
```

Add `use serde::Serialize;`. Register in `src/api/mod.rs` after the list route:

```rust
.route("/api/failures/uncovered", get(failures::uncovered))
```

- [ ] **Step 6: Write the endpoint tests**

Append to `tests/api_test.rs`, reusing `app_with_recorder()` from Task 3:

```rust
#[tokio::test]
async fn uncovered_reports_the_count_for_the_requested_bracket() {
    let (app, recorder) = app_with_recorder().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures/uncovered?instance=dns-node-2&to=1761436800000&windowMinutes=120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap(),
    )
    .unwrap();
    assert_eq!(body["unlabeled"], 0);

    let filter = recorder.last_filter.lock().unwrap().clone().unwrap();
    assert_eq!(filter.instance.as_deref(), Some("dns-node-2"));
    assert_eq!(filter.range, Range::new(1_761_429_600_000, 1_761_436_800_000));
}

#[tokio::test]
async fn uncovered_is_not_matched_by_the_trace_route() {
    // `/api/failures/{trace_id}` must not answer this literal path: the detail
    // handler would look up a trace named "uncovered" and miss.
    let (app, _recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/failures/uncovered")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap())
            .unwrap();
    assert!(body.get("unlabeled").is_some(), "unexpected body: {body}");
}

#[tokio::test]
async fn uncovered_rejects_a_too_long_bracket() {
    let now_ms = 1_761_436_800_000;
    let (app, _recorder) = app_with_recorder().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/api/failures/uncovered?from={}&to={}",
                    now_ms - 7 * 1440 * 60_000 - 1,
                    now_ms
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

The recording telemetry source in `tests/api_test.rs` does not override `unlabeled_failures`, so it answers through the trait default and the first test asserts `0` — that is the point of the default. Add `use serde_json::Value;` if the file does not already import it.

- [ ] **Step 7: Run the backend suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS.

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
feat: filter failures by host equality and count what it cannot reach

lower(host_name) = lower(?) narrows the list; GET /api/failures/uncovered
answers how many in-scope failures carry no host label, because the equality
silently drops them and an empty list would otherwise look like a healthy node.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 5: Decouple the fingerprint from the requested bracket (D11)

This is the load-bearing change of the round: with the range inside the fingerprint, dragging the window from 1h to 7d invalidates a cached analysis and re-burns the model budget on identical evidence. Write the fingerprint pin first and watch it fail.

**Files:**
- Modify: `src/evidence/model.rs:42-59` (`EvidenceFilter`)
- Modify: `src/analysis/prompt.rs:208` (the `filter:` legend) and `:215`
- Modify: `src/api/analysis.rs:77-80` (the `AnalysisSnapshot.filter` doc)
- Test: `tests/evidence_test.rs`, `tests/api_evidence_test.rs`, `tests/api_analysis_test.rs`, `tests/analysis_test.rs`

**Interfaces:**
- Consumes: `FailureFilter.instance` (Task 2), `EvidenceFilter::from(&FailureFilter)` (Task 2's interim bridge).
- Produces: `EvidenceFilter { domain: Option<String>, response_code: Option<String>, instance: Option<String> }` — no range field at all. Task 8 mirrors it in TypeScript.

- [ ] **Step 1: Write the failing fingerprint pin**

Append to `tests/evidence_test.rs`, reusing `run_assemble` (`:1495-1501`), `bundle_telemetry`-style fakes, `GENERATED_AT` (`:432`), `window_metrics()` and `FakeInstances::new(resolve_match(...), calls.clone())`:

```rust
#[tokio::test]
async fn assemble_produces_the_same_fingerprint_for_a_one_hour_and_a_week_bracket() {
    // The D11 core pin. Logs and spans are matched by trace id and the metric
    // window is centred on the trace itself, so widening the requested bracket
    // changes no evidence — and must not invalidate the cached analysis.
    let calls = Calls::new();
    let telemetry = FakeTelemetry::new(
        fake_detail(Some(group_for(TRACE_ID)), vec![], vec![]),
        true,
        Some(window_metrics()),
        calls.clone(),
    );
    let instances = FakeInstances::new(resolve_match(MatchQuality::Exact), calls.clone());

    let one_hour = run_assemble(&telemetry, &instances, TRACE_ID, &bracket(60), &FixedClock(GENERATED_AT))
        .await
        .unwrap();
    let one_week =
        run_assemble(&telemetry, &instances, TRACE_ID, &bracket(7 * 1440), &FixedClock(GENERATED_AT))
            .await
            .unwrap();

    assert_eq!(one_hour.fingerprint, one_week.fingerprint);
    assert_eq!(one_hour.logs.items, one_week.logs.items);
    assert_eq!(one_hour.metrics, one_week.metrics);
}

#[tokio::test]
async fn evidence_filter_no_longer_echoes_the_requested_span() {
    let calls = Calls::new();
    let telemetry = FakeTelemetry::new(
        fake_detail(Some(group_for(TRACE_ID)), vec![], vec![]),
        true,
        Some(window_metrics()),
        calls.clone(),
    );
    let instances = FakeInstances::new(resolve_match(MatchQuality::Exact), calls.clone());

    let bundle = run_assemble(
        &telemetry,
        &instances,
        TRACE_ID,
        &bracket(7 * 1440),
        &FixedClock(GENERATED_AT),
    )
    .await
    .unwrap();

    assert_eq!(bundle.filter.domain, None);
    assert_eq!(bundle.filter.response_code, None);
    assert_eq!(bundle.filter.instance, None);
}

#[tokio::test]
async fn evidence_filter_still_echoes_domain_response_code_and_instance() {
    let calls = Calls::new();
    let telemetry = FakeTelemetry::new(
        fake_detail(Some(group_for(TRACE_ID)), vec![], vec![]),
        true,
        Some(window_metrics()),
        calls.clone(),
    );
    let instances = FakeInstances::new(resolve_match(MatchQuality::Exact), calls.clone());

    let filter = FailureFilter {
        range: Range::new(GENERATED_AT - 60 * 60_000, GENERATED_AT),
        domain: Some("api.internal".to_owned()),
        response_code: Some("REFUSED".to_owned()),
        instance: Some("dns-node-2".to_owned()),
        limit: 1,
    };

    let bundle = run_assemble(&telemetry, &instances, TRACE_ID, &filter, &FixedClock(GENERATED_AT))
        .await
        .unwrap();

    assert_eq!(bundle.filter.domain.as_deref(), Some("api.internal"));
    assert_eq!(bundle.filter.response_code.as_deref(), Some("REFUSED"));
    assert_eq!(bundle.filter.instance.as_deref(), Some("dns-node-2"));
}

#[tokio::test]
async fn changing_the_instance_filter_changes_the_fingerprint() {
    // The three echoed fields are the only filter inputs the analyst sees, so
    // each must still be able to mark a cached analysis stale.
    let quality_changed = |instance: Option<String>| {
        let calls = Calls::new();
        let telemetry = FakeTelemetry::new(
            fake_detail(Some(group_for(TRACE_ID)), vec![], vec![]),
            true,
            Some(window_metrics()),
            calls.clone(),
        );
        let instances = FakeInstances::new(resolve_match(MatchQuality::Exact), calls);
        let mut filter = assemble_filter();
        filter.instance = instance;
        async move {
            run_assemble(&telemetry, &instances, TRACE_ID, &filter, &FixedClock(GENERATED_AT))
                .await
                .unwrap()
                .fingerprint
        }
    };

    assert_ne!(
        quality_changed(None).await,
        quality_changed(Some("dns-node-2".to_owned())).await
    );
}

/// A bracket of `minutes` ending at the fixture clock.
fn bracket(minutes: i64) -> FailureFilter {
    FailureFilter {
        range: Range::new(GENERATED_AT - minutes * 60_000, GENERATED_AT),
        ..assemble_filter()
    }
}
```

`group_for(TRACE_ID)` names the existing group helper in the file — if the fakes build the group inline, reuse whatever helper already returns a `FailureGroup` for `TRACE_ID` (`:431`), and keep `assert_fingerprint_changes` (`:644`) working. If the file has no such helper, build the same `FailureDetail` the neighbouring assemble tests already use; the pin is about the two fingerprints, not the group contents.

- [ ] **Step 2: Also pin the prompt legend**

Append to `tests/analysis_test.rs`, next to the existing `system_prompt(&prompt_facts())` assertions (`:327`, `:342`, `:449`):

```rust
#[test]
fn system_prompt_describes_the_range_free_filter_echo() {
    let prompt = system_prompt(&prompt_facts());

    assert!(
        prompt.contains("{{domain, responseCode, instance}}"),
        "the analyst is told the wrong filter shape: {prompt}"
    );
    assert!(
        prompt.contains("does not carry the requested time bracket"),
        "the analyst is not told the echo is range-free"
    );
    assert!(
        !prompt.contains("windowMinutes"),
        "the stale window legend is still promised to the analyst"
    );
}
```

- [ ] **Step 3: Run them to confirm they fail**

Run: `cargo test --test evidence_test fingerprint 2>&1 | tail -30 && cargo test --test analysis_test range_free 2>&1 | tail -20`
Expected: FAIL — the two brackets produce different fingerprints (the echo still carries the span) and the prompt still says `windowMinutes`.

- [ ] **Step 4: Reshape `EvidenceFilter`**

Replace `src/evidence/model.rs:42-59` with:

```rust
/// Filter echo so an analysis can state which slice of telemetry it describes.
///
/// Deliberately free of the requested bracket: logs and spans are matched by
/// `trace_id` and the metric window is centred on the trace itself
/// (`METRICS_RADIUS_MS`), so the evidence set does not depend on how wide a
/// window the user framed. Recording the range here would turn "widen the
/// window to a week" into a cache miss and a re-burn of the tool budget on
/// byte-identical evidence. The range's real effect still shows up where it
/// belongs: `logs.total`/`omitted`/`truncated`, `missing`, and the timestamps
/// of the log rows themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceFilter {
    pub domain: Option<String>,
    pub response_code: Option<String>,
    pub instance: Option<String>,
}

impl From<&FailureFilter> for EvidenceFilter {
    fn from(filter: &FailureFilter) -> Self {
        Self {
            domain: filter.domain.clone(),
            response_code: filter.response_code.clone(),
            instance: filter.instance.clone(),
        }
    }
}
```

- [ ] **Step 5: Update the analyst legend**

`src/analysis/prompt.rs:208` becomes:

```
- filter: the telemetry slice as {{domain, responseCode, instance}}. It does not carry the requested time bracket; the bracket is reflected in the log timestamps, logs.total/omitted/truncated, and missing.
```

and `:215` — "or null when the window has no metrics" — becomes `- metrics: metric evidence, or null when the trace has no metrics.` so the text no longer implies a requested window drives it.

- [ ] **Step 6: Update the snapshot doc**

`src/api/analysis.rs:77-80` documents that `filter` is `{windowMinutes, domain, responseCode}`. Rewrite it to `{domain, responseCode, instance}` and add one sentence: the bracket is not part of the fingerprint, so reusing a stored analysis after widening the window is intended, not staleness.

- [ ] **Step 7: Fix the echoes that still name the old shape**

`tests/api_evidence_test.rs:99` (`body["filter"]["windowMinutes"] == 60`) → assert the new trio:

```rust
assert_eq!(body["filter"]["domain"], serde_json::Value::Null);
assert_eq!(body["filter"]["responseCode"], serde_json::Value::Null);
assert_eq!(body["filter"]["instance"], serde_json::Value::Null);
assert!(body["filter"].get("windowMinutes").is_none());
```

Keep the `:156-157` domain/responseCode echo case and make it assert `instance` too. In `tests/evidence_test.rs`: delete the `window_minutes` line from `bundle()`'s literal (`:604-611`, the struct at `:608`), the echo equality literals (`:1559-1566`, `:2305-2312`), and remove `assert_fingerprint_changes(|bundle| bundle.filter.window_minutes = 30, "windowMinutes")` (`:835`) — replace it with `assert_fingerprint_changes(|bundle| bundle.filter.instance = Some("dns-node-2".to_owned()), "instance")`. `tests/api_analysis_test.rs:266` and any other `windowMinutes` echo assertion follow the same substitution.

- [ ] **Step 8: Run the backend suite**

Run: `cargo test 2>&1 | tail -40`
Expected: PASS, including the new equal-fingerprint pin.

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
docs: decouple the evidence fingerprint from the requested range

The echo now carries domain/responseCode/instance only. Logs and spans match by
trace id and the metric window is trace-centred, so widening 1h -> 7d keeps the
same fingerprint and the cached analysis stays reusable instead of re-burning
the tool budget on identical evidence.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 6: The copilot sees the same contract as the browser

D9: `list_failures` and `get_failure_detail` accept `from`/`to`/`instance` and their **descriptions** state the real limits — a description promising a 24-hour ceiling is a lie the model will act on. `get_overview` stays relative-only (deviation 2).

**Files:**
- Modify: `src/agent/tools.rs:19-35` (`FailureArgs`, `TraceArgs`), `:60-86` (both handlers), `:116-136` (both descriptions)
- Test: `tests/tools_test.rs`, `tests/agent_test.rs`

**Interfaces:**
- Consumes: `Range::resolve` (Task 1), `investigation_filter`'s precedence semantics (Task 3) — the tools call `Range::resolve` directly rather than the HTTP-layer helper, because they have no query string; `DataError::Invalid` for rejection.
- Produces: `FailureArgs { window_minutes, from, to, domain, response_code, instance, limit }`, `TraceArgs { trace_id, window_minutes, from, to, domain, response_code, instance }`. Task 13's README text must agree with the ceilings stated here.

- [ ] **Step 1: Write the failing description and behaviour pins**

Append to `tests/tools_test.rs`. The file builds a fixture service at `:8-13`; give `fixture_service()` a single shared clock so the service and the fixture agree (Task 2 already changed the constructor signature):

```rust
fn fixture_service() -> ConsoleService {
    let clock = Arc::new(FixedClock(1_761_436_800_000));
    let fixture = Arc::new(FixtureSource::new(clock.clone()));
    let telemetry = fixture.clone() as Arc<dyn TelemetrySource>;
    let instances = fixture as Arc<dyn InstanceSource>;
    ConsoleService::new(telemetry, instances, clock)
}
```

Then the pins:

```rust
#[tokio::test]
async fn failure_tool_descriptions_state_the_absolute_range_and_the_real_ceilings() {
    let service = fixture_service();
    let tools = tools::build_tools(Arc::new(service.clone()));

    let list = tools
        .iter()
        .find(|tool| tool.name() == "list_failures")
        .expect("list_failures");
    let detail = tools
        .iter()
        .find(|tool| tool.name() == "get_failure_detail")
        .expect("get_failure_detail");
    let overview = tools
        .iter()
        .find(|tool| tool.name() == "get_overview")
        .expect("get_overview");

    let list_text = list.description();
    let detail_text = detail.description();
    for text in [&list_text, &detail_text] {
        assert!(text.contains("from"), "{text}");
        assert!(text.contains("to"), "{text}");
        assert!(text.contains("instance"), "{text}");
        assert!(text.contains("10080") || text.contains("7 days"), "{text}");
        assert!(text.contains("epoch milliseconds"), "{text}");
    }
    assert!(
        detail_text.contains("same from/to/domain/responseCode/instance"),
        "the detail tool no longer tells the model to match the list call: {detail_text}"
    );
    // The live class keeps its 1440-minute ceiling and must not promise a week.
    assert!(
        !overview.description().contains("10080"),
        "get_overview advertises the investigation ceiling: {}",
        overview.description()
    );
}

#[tokio::test]
async fn list_failures_resolves_an_absolute_bracket_and_rejects_an_impossible_one() {
    let service = fixture_service();

    let value = tools::list_failures(
        &service,
        FailureArgs {
            from: Some(1_761_436_800_000 - 3 * 24 * 60 * 60_000),
            to: Some(1_761_436_800_000),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap();
    assert!(value.is_array(), "expected a group array, got {value}");

    let error = tools::list_failures(
        &service,
        FailureArgs {
            from: Some(1_761_436_800_000 - 8 * 24 * 60 * 60_000),
            to: Some(1_761_436_800_000),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, DataError::Invalid(_)),
        "expected a rejection, got {error:?}"
    );
}

#[tokio::test]
async fn failure_tools_pass_the_instance_to_the_filter() {
    let service = fixture_service();

    let filtered = tools::list_failures(
        &service,
        FailureArgs {
            from: Some(1_761_436_800_000 - 7 * 1440 * 60_000),
            to: Some(1_761_436_800_000),
            instance: Some("dns-node-3".to_owned()),
            ..FailureArgs::default()
        },
    )
    .await
    .unwrap();
    let groups = filtered.as_array().expect("group array");
    assert!(
        groups
            .iter()
            .all(|group| group["instance"].as_str() == Some("dns-node-3")),
        "instance filter leaked other hosts: {filtered}"
    );
}
```

Add `use doggy_dns_console::agent::tools::FailureArgs;` and `use doggy_dns_console::error::DataError;` to the imports. The last test only passes once Task 7's fixture spread and host filtering exist; if Task 7 lands later, assert that every returned group carries the requested host and let Task 7 keep it honest.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test tools_test 2>&1 | tail -30`
Expected: FAIL — compile error on the new `FailureArgs` fields, then the description assertions.

- [ ] **Step 3: Extend the argument structs**

`src/agent/tools.rs:19-35` becomes:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailureArgs {
    /// Relative shortcut; expanded to `[to - n, to]`. Ignored when both `from`
    /// and `to` are given.
    pub window_minutes: Option<u32>,
    /// Absolute bracket start in epoch milliseconds.
    pub from: Option<i64>,
    /// Absolute bracket end in epoch milliseconds; defaults to now.
    pub to: Option<i64>,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    /// Case-insensitive host equality; omit to cover every host.
    pub instance: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceArgs {
    pub trace_id: String,
    pub window_minutes: Option<u32>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub domain: Option<String>,
    pub response_code: Option<String>,
    pub instance: Option<String>,
}
```

The schemars-derived parameter schema follows these fields automatically, which is what makes the `description()` pin meaningful rather than decorative.

- [ ] **Step 4: Resolve the bracket in both handlers**

Replace the Task 2 bridge in `list_failures` (`src/agent/tools.rs:60-72`) and `get_failure_detail` (`:74-86`) with the real resolution — same precedence, same rejection list, propagated instead of swallowed:

```rust
pub async fn list_failures(
    service: &ConsoleService,
    args: FailureArgs,
) -> Result<Value, DataError> {
    let range = Range::resolve(args.from, args.to, args.window_minutes, service.now_ms())
        .map_err(DataError::Invalid)?;
    let filter = FailureFilter {
        range,
        domain: args.domain,
        response_code: args.response_code,
        instance: args.instance,
        limit: args.limit.unwrap_or(50).clamp(1, 200),
    };
    let groups = service.failures(filter).await?;
    serde_json::to_value(groups).map_err(|error| DataError::Invalid(error.to_string()))
}
```

`get_failure_detail` is identical except `limit: 1` and `args.trace_id` in the `service.failure_detail(...)` call. Update the import at `src/agent/tools.rs:9` to `use crate::domain::{FailureFilter, Range, Window};` (`Window` is still needed by `get_overview`); `DEFAULT_WINDOW_MINUTES` no longer needs importing once the bridge is gone.

- [ ] **Step 5: Rewrite the descriptions**

`src/agent/tools.rs:118` (`list_failures`) becomes:

```rust
"List DNS failures (SERVFAIL / REFUSED / FORMERR / timeouts) grouped by trace_id. NXDOMAIN is a normal negative answer and is always excluded. Frame the window with from/to in epoch milliseconds (span up to 7 days, i.e. 10080 minutes) or with the windowMinutes shortcut; a bracket under 5 minutes, over 10080 minutes, with start at or after end, or ending more than 60 seconds in the future is rejected. Set instance to a host name to restrict to that node; rows without a host label are then excluded.",
```

`src/agent/tools.rs:129` (`get_failure_detail`) becomes:

```rust
"Return the failure group, logs and spans for one trace_id so the analysis can cite real evidence. Pass the same from/to/domain/responseCode/instance used by list_failures so the detail cannot drift from the row the operator clicked.",
```

`get_overview` (`:107`) gains one clarifying sentence so the two ceilings are not confused by the model:

```rust
"Return DNS overview: instances online/total, query volume, failure count, success rate, p95 latency and the series. Use this before quoting any KPI. The overview only accepts the relative windowMinutes shortcut and clamps it to 5-1440 minutes; for a longer look-back use list_failures.",
```

- [ ] **Step 6: Run the backend suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. `tests/agent_test.rs` and `tests/agui_test.rs` build their own fakes and constructor calls; Task 2 already migrated their `ConsoleService::new` arity, so nothing here should move.

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
feat: give the copilot failure tools the absolute bracket and host filter

list_failures and get_failure_detail resolve from/to/windowMinutes with the same
precedence and rejection list as the HTTP endpoints, and their descriptions now
state the 10080-minute ceiling instead of leaving the model to guess. get_overview
keeps the 5-1440 relative window and says so.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 7: The demo fixture really filters, and really spreads its rows

D10: without this, the only end-to-end evidence for the whole round is self-confirmation. Five rows all sitting in `now-30min..now-60s` means "1h" and "7d" return identical lists and no assertion can distinguish a working filter from a no-op.

**Files:**
- Modify: `src/datasource/fixture.rs:22-94` (`groups()`), `:122-139` (`failures()`), `:141-200` (`failure_detail()`), and the `TelemetrySource::unlabeled_failures` override
- Test: `tests/fixture_test.rs`, `tests/evidence_test.rs:2290-2326`

**Interfaces:**
- Consumes: `FailureFilter { range, instance, domain, response_code, limit }` (Tasks 2-4).
- Produces: eight fixture groups with the ages and hosts listed below; `fn matches(&self, group: &FailureGroup, filter: &FailureFilter) -> bool` inside `FixtureSource`; `unlabeled_failures` returning the count of in-scope rows with no host.

The table this task builds (all ages relative to the fixture clock; `first_seen = now - (age + 10) minutes`, `last_seen = now - age`):

| trace | query | code | instance | count | age |
| --- | --- | --- | --- | --- | --- |
| `4bf92f3577b34da6` | `user-service.DEFAULT_GROUP.public.nacos.local.` | SERVFAIL | `dns-node-1` | 12 | 15 min |
| `5cf92f3577b34da7` | `payment-service.DEFAULT_GROUP.public.nacos.local.` | SERVFAIL | `dns-node-2` | 7 | 3 h |
| `6df92f3577b34da8` | `missing.example.com.` | NXDOMAIN | `dns-node-1` | 31 | 45 min |
| `7ef92f3577b34da9` | `api.internal.` | REFUSED | `dns-node-3` | 3 | 3 d |
| `8ff92f3577b34da0` | `slow.example.com.` | SERVFAIL | `dns-node-2` | 2 | 10 min |
| `9af92f3577b34db1` | `batch.internal.` | SERVFAIL | `dns-node-2` | 4 | 6 d |
| `a0f92f3577b34db2` | `old.internal.` | SERVFAIL | `dns-node-1` | 9 | 8 d |
| `b1f92f3577b34db3` | `unlabeled.example.com.` | SERVFAIL | *none* | 5 | 20 min |

Why these numbers: `4bf9` stays the top row inside a default 60-minute bracket (count 12 beats the 8-day `a0f9` only when the bracket is widened, and `a0f9` is unreachable anyway), NXDOMAIN stays excluded, `responseCode=REFUSED` for `4bf9` still misses, `instance=dns-node-1` with a 60-minute bracket still yields `recent_failures = 12` for the instance page, and `b1f9` is the unlabeled witness the notice needs.

- [ ] **Step 1: Write the failing filter tests**

Replace the two `FailureFilter` literals in `tests/fixture_test.rs` (`:29-33`, `:39-43`) and append:

```rust
fn bracket(minutes: i64) -> FailureFilter {
    let end = 1_761_436_800_000;
    FailureFilter {
        range: Range::new(end - minutes * 60_000, end),
        ..FailureFilter::new(Range::default())
    }
}

#[tokio::test]
async fn failures_narrow_to_the_requested_bracket() {
    let service = service();

    let one_hour = service
        .failures(bracket(60))
        .await
        .unwrap()
        .into_iter()
        .map(|group| group.trace_id.unwrap())
        .collect::<Vec<_>>();
    let one_week = service
        .failures(bracket(7 * 1440))
        .await
        .unwrap()
        .into_iter()
        .map(|group| group.trace_id.unwrap())
        .collect::<Vec<_>>();

    assert_eq!(one_hour, ["4bf92f3577b34da6", "b1f92f3577b34db3", "8ff92f3577b34da0"]);
    // 5cf9 (3h) and 7ef9 (3d) only appear once the bracket widens; a0f9 (8d)
    // never does.
    assert!(one_week.contains(&"5cf92f3577b34da7".to_owned()));
    assert!(one_week.contains(&"7ef92f3577b34da9".to_owned()));
    assert!(one_week.contains(&"9af92f3577b34db1".to_owned()));
    assert!(!one_week.contains(&"a0f92f3577b34db2".to_owned()));
    assert!(!one_week.contains(&"6df92f3577b34da8".to_owned()));
    assert!(one_week.len() > one_hour.len());
}

#[tokio::test]
async fn failures_narrow_to_the_instance_case_insensitively() {
    let service = service();

    let filter = FailureFilter {
        instance: Some("DNS-NODE-2".to_owned()),
        ..bracket(7 * 1440)
    };
    let groups = service.failures(filter).await.unwrap();
    let traces: Vec<_> = groups.iter().map(|group| group.trace_id.clone().unwrap()).collect();

    assert_eq!(traces, ["5cf92f3577b34da7", "9af92f3577b34db1", "8ff92f3577b34da0"]);
    // The unlabeled row is unreachable by a host equality, in the fixture exactly
    // as it is in OpenObserve.
    assert!(!traces.contains(&"b1f92f3577b34db3".to_owned()));
}

#[tokio::test]
async fn failures_report_unlabeled_rows_the_instance_filter_cannot_reach() {
    let service = service();

    let filter = FailureFilter {
        instance: Some("dns-node-2".to_owned()),
        ..bracket(7 * 1440)
    };
    assert_eq!(service.unlabeled_failures(filter).await.unwrap(), 1);

    // Out of the bracket, out of the count.
    let narrow = FailureFilter {
        instance: Some("dns-node-2".to_owned()),
        ..bracket(15)
    };
    assert_eq!(service.unlabeled_failures(narrow).await.unwrap(), 0);
}

#[tokio::test]
async fn failure_detail_respects_the_bracket() {
    let service = service();

    // 7ef9 happened 3 days ago: visible in a week, a 404 inside an hour.
    let wide = service
        .failure_detail("7ef92f3577b34da9", bracket(7 * 1440))
        .await;
    assert!(wide.is_ok());

    let error = service
        .failure_detail("7ef92f3577b34da9", bracket(60))
        .await
        .unwrap_err();
    assert!(matches!(error, DataError::NotFound(_)), "got {error:?}");
}
```

Add `use doggy_dns_console::domain::Range;` and `use doggy_dns_console::error::DataError;` to `tests/fixture_test.rs`. The `service()` helper there already builds `ConsoleService::new` over `FixtureSource::new(Arc::new(FixedClock(1_761_436_800_000)))`; Task 2 makes it pass that same clock into the service, so `now_ms()` and the fixture agree.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test --test fixture_test 2>&1 | tail -30`
Expected: FAIL — the current fixture ignores `range` and `instance`, so every narrowing assertion above is red.

- [ ] **Step 3: Spread the rows**

Rewrite `groups()` (`src/datasource/fixture.rs:22-94`) so each tuple carries its age in minutes:

```rust
fn groups(&self) -> Vec<FailureGroup> {
    let now = self.clock.now_ms();
    let rows = [
        (
            "4bf92f3577b34da6",
            "user-service.DEFAULT_GROUP.public.nacos.local.",
            "SERVFAIL",
            "upstream timed out",
            12usize,
            Some("dns-node-1"),
            "nacos[nacos.local]",
            15i64,
        ),
        (
            "5cf92f3577b34da7",
            "payment-service.DEFAULT_GROUP.public.nacos.local.",
            "SERVFAIL",
            "upstream connection refused",
            7,
            Some("dns-node-2"),
            "forward[8.8.8.8:53]",
            180,
        ),
        (
            "6df92f3577b34da8",
            "missing.example.com.",
            "NXDOMAIN",
            "no authority handled query",
            31,
            Some("dns-node-1"),
            "none",
            45,
        ),
        (
            "7ef92f3577b34da9",
            "api.internal.",
            "REFUSED",
            "query refused by policy",
            3,
            Some("dns-node-3"),
            "native",
            3 * 1440,
        ),
        (
            "8ff92f3577b34da0",
            "slow.example.com.",
            "SERVFAIL",
            "upstream timeout after 5s",
            2,
            Some("dns-node-2"),
            "forward[1.1.1.1:53]",
            10,
        ),
        (
            "9af92f3577b34db1",
            "batch.internal.",
            "SERVFAIL",
            "upstream reset",
            4,
            Some("dns-node-2"),
            "forward[9.9.9.9:53]",
            6 * 1440,
        ),
        (
            "a0f92f3577b34db2",
            "old.internal.",
            "SERVFAIL",
            "upstream timed out",
            9,
            Some("dns-node-1"),
            "nacos[nacos.local]",
            8 * 1440,
        ),
        (
            "b1f92f3577b34db3",
            "unlabeled.example.com.",
            "SERVFAIL",
            "upstream timed out",
            5,
            None,
            "forward[8.8.8.8:53]",
            20,
        ),
    ];

    rows.into_iter()
        .map(|(trace, query, code, error, count, instance, authority, age_minutes)| {
            FailureGroup {
                key: format!("trace:{trace}"),
                trace_id: Some(trace.to_string()),
                query: query.to_string(),
                qtype: "A".to_string(),
                instance: instance.map(str::to_string),
                authority: Some(authority.to_string()),
                response_code: code.to_string(),
                error: Some(error.to_string()),
                count: count as u64,
                // Ten minutes of spread so the group is a window, not a point:
                // a bracket ending mid-group still overlaps it.
                first_seen: now - (age_minutes + 10) * 60_000,
                last_seen: now - age_minutes * 60_000,
                duration_ms: if code == "SERVFAIL" { Some(5012) } else { Some(2) },
            }
        })
        .collect()
}
```

- [ ] **Step 4: Filter for real**

Add the shared predicate to `impl FixtureSource` (after `groups()`), then use it in `failures()`:

```rust
/// Demo-side stand-in for the OpenObserve WHERE clause: overlap on the bracket,
/// substring on the domain, equality on the response code, host equality on the
/// instance. An unlabeled group is never reachable by the host filter.
fn matches(&self, group: &FailureGroup, filter: &FailureFilter) -> bool {
    if group.response_code.eq_ignore_ascii_case("NXDOMAIN") {
        return false;
    }
    if group.last_seen < filter.range.start_ms || group.first_seen > filter.range.end_ms {
        return false;
    }
    if let Some(domain) = &filter.domain {
        if !group.query.contains(domain.as_str()) {
            return false;
        }
    }
    if let Some(code) = &filter.response_code {
        if !group.response_code.eq_ignore_ascii_case(code) {
            return false;
        }
    }
    if let Some(instance) = &filter.instance {
        let Some(host) = &group.instance else {
            return false;
        };
        if !host.eq_ignore_ascii_case(instance) {
            return false;
        }
    }
    true
}
```

`failures()` (`src/datasource/fixture.rs:122-139`) keeps its sort and truncate but replaces the `retain` calls:

```rust
async fn failures(&self, filter: FailureFilter) -> Result<Vec<FailureGroup>, DataError> {
    let mut groups = self.groups();
    groups.retain(|group| self.matches(group, &filter));
    groups.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| right.last_seen.cmp(&left.last_seen))
    });
    groups.truncate(filter.limit.clamp(1, 200));
    Ok(groups)
}
```

`failure_detail()` (`:141-200`) replaces its inline condition chain with the same predicate — and keeps the log timestamp inside the group's own window so the drawer's log row is not in the future relative to `first_seen`:

```rust
let group = self
    .groups()
    .into_iter()
    .find(|group| group.trace_id.as_deref() == Some(trace_id) && self.matches(group, &filter))
    .ok_or_else(|| DataError::NotFound(format!("failure trace {trace_id}")))?;
```

and the single `LogRecord`'s `ts` becomes `group.last_seen + 5 * 60_000` instead of `now - 60 * 1000`. Add the override after `failure_detail`:

```rust
async fn unlabeled_failures(&self, filter: FailureFilter) -> Result<u64, DataError> {
    let unlabeled = self
        .groups()
        .into_iter()
        .filter(|group| group.instance.is_none())
        .filter(|group| {
            // Same scope as the list, minus the host equality that cannot match.
            let mut scope = filter.clone();
            scope.instance = None;
            self.matches(group, &scope)
        })
        .count();
    Ok(unlabeled as u64)
}
```

- [ ] **Step 5: Run the backend suite**

Run: `cargo test 2>&1 | tail -40`
Expected: PASS. Watch these specifically, because the spread can reach them: `tests/fixture_test.rs` (the `groups[0].trace_id` round trip is still `4bf92f3577b34da6` — count 12 is the largest in-scope row for any bracket that reaches it), `tests/api_evidence_test.rs` (the fixture-backed evidence trace `4bf92f3577b34da6` stays inside the default relative hour), `tests/evidence_test.rs:2290-2326` (the wide `assemble_filter()` bracket keeps every row reachable), and `tests/tools_test.rs` (Task 6's instance pin now has real data behind it).

Run: `cargo fmt && cargo clippy --all-targets -- --D warnings 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add -A src tests
git commit -m "$(cat <<'EOF'
feat: make the demo fixture honour the bracket, host and unlabeled counts

Rows previously all sat in the last 30 minutes, so "1h" and "7d" returned the
same list and no assertion could tell a real filter from a no-op. Ages now span
10 minutes to 8 days across three hosts, plus one host-less row that only the
uncovered count can see.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 8: The frontend filter contract, as one pure module

Everything the page and the drawer need to agree on — precedence, validation, URL round-trip, labels — lives in `src/lib/failureFilter.ts` with no Vue dependency, so Tasks 9-12 stay thin and this logic is unit-testable without a mount.

**Files:**
- Create: `doggy-dns-ui/src/lib/failureFilter.ts`
- Create: `doggy-dns-ui/src/lib/__tests__/failureFilter.spec.ts`
- Modify: `doggy-dns-ui/src/api/types.ts:134-138` (`EvidenceFilter`) and add `InvestigationFilter`

**Interfaces:**
- Consumes: the backend wire shape from Tasks 3-5 (`from`/`to`/`windowMinutes`/`domain`/`responseCode`/`instance`, camelCase; `EvidenceFilter = { domain, responseCode, instance }`).
- Produces: `FilterDraft`, `Bracket`, `MIN_RANGE_MINUTES`, `MAX_RANGE_MINUTES`, `FUTURE_TOLERANCE_MS`, `emptyDraft`, `relativeDraft`, `isAbsolute`, `parseFilterQuery`, `toRouteQuery`, `resolveBracket`, `validateBracket`, `toRequestQuery`, `toInvestigationFilter`, `datetimeToMs`, `msToDatetime`, `rangeLabel`. Task 9 consumes `FilterDraft` + `toRequestQuery` + `validateBracket`; Task 10 consumes `toInvestigationFilter`; Task 11 consumes `parseFilterQuery`/`toRouteQuery`/`rangeLabel`; Task 12 consumes `relativeDraft`.

- [ ] **Step 1: Write the failing module tests**

Create `doggy-dns-ui/src/lib/__tests__/failureFilter.spec.ts`:

```ts
import { describe, expect, it } from 'vitest'
import {
  FUTURE_TOLERANCE_MS,
  MAX_RANGE_MINUTES,
  MIN_RANGE_MINUTES,
  datetimeToMs,
  emptyDraft,
  isAbsolute,
  msToDatetime,
  parseFilterQuery,
  rangeLabel,
  relativeDraft,
  resolveBracket,
  toInvestigationFilter,
  toRequestQuery,
  toRouteQuery,
  validateBracket,
} from '../failureFilter'

const NOW = 1_761_436_800_000
const HOUR = 3_600_000

describe('resolveBracket', () => {
  it('lets from/to win over windowMinutes', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 10 * HOUR, toMs: NOW, windowMinutes: 60 }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 10 * HOUR, endMs: NOW })
  })

  it('expands a relative window backwards from to', () => {
    const draft = { ...emptyDraft(), toMs: NOW, windowMinutes: 120 }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 2 * HOUR, endMs: NOW })
  })

  it('defaults to a relative hour anchored at now', () => {
    const draft = emptyDraft()
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - HOUR, endMs: NOW })
  })

  it('keeps a lone from bound and derives the end from the window', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: null, windowMinutes: null }
    expect(resolveBracket(draft, NOW)).toEqual({ startMs: NOW - 3 * HOUR, endMs: NOW })
  })
})

describe('validateBracket', () => {
  it('accepts the legal edges', () => {
    expect(validateBracket({ startMs: NOW - MIN_RANGE_MINUTES * 60_000, endMs: NOW }, NOW)).toBeNull()
    expect(validateBracket({ startMs: NOW - MAX_RANGE_MINUTES * 60_000, endMs: NOW }, NOW)).toBeNull()
    expect(
      validateBracket({ startMs: NOW - HOUR, endMs: NOW + FUTURE_TOLERANCE_MS }, NOW),
    ).toBeNull()
  })

  it('rejects the four documented rules', () => {
    expect(validateBracket({ startMs: NOW, endMs: NOW }, NOW)).toMatch(/before/)
    expect(validateBracket({ startMs: NOW + HOUR, endMs: NOW }, NOW)).toMatch(/before/)
    expect(validateBracket({ startMs: NOW - 4 * 60_000, endMs: NOW }, NOW)).toMatch(/5/)
    expect(validateBracket({ startMs: NOW - MAX_RANGE_MINUTES * 60_000 - 1, endMs: NOW }, NOW)).toMatch(
      /10080/,
    )
    expect(validateBracket({ startMs: NOW - HOUR, endMs: NOW + FUTURE_TOLERANCE_MS + 1 }, NOW)).toMatch(
      /不能超过当前时间/,
    )
  })

  it('never clamps a 7-day bracket back to a day', () => {
    const bracket = { startMs: NOW - MAX_RANGE_MINUTES * 60_000, endMs: NOW }
    expect(validateBracket(bracket, NOW)).toBeNull()
    expect(bracket.endMs - bracket.startMs).toBe(MAX_RANGE_MINUTES * 60_000)
  })
})

describe('query round-trip', () => {
  it('parses the documented list URL', () => {
    const draft = parseFilterQuery({
      from: String(NOW - 3 * HOUR),
      to: String(NOW),
      domain: 'nacos',
      responseCode: 'SERVFAIL',
      instance: 'dns-node-2',
    })
    expect(draft.fromMs).toBe(NOW - 3 * HOUR)
    expect(draft.toMs).toBe(NOW)
    expect(draft.domain).toBe('nacos')
    expect(draft.responseCode).toBe('SERVFAIL')
    expect(draft.instance).toBe('dns-node-2')
    expect(isAbsolute(draft)).toBe(true)
  })

  it('parses a relative shortcut', () => {
    const draft = parseFilterQuery({ windowMinutes: '1440' })
    expect(draft.windowMinutes).toBe(1440)
    expect(isAbsolute(draft)).toBe(false)
  })

  it('drops non-numeric bounds instead of guessing', () => {
    const draft = parseFilterQuery({ from: 'abc', to: '1' })
    expect(draft.fromMs).toBeNull()
    expect(draft.toMs).toBeNull()
  })

  it('writes a relative draft as windowMinutes only', () => {
    const query = toRouteQuery(relativeDraft(60))
    expect(query).toEqual({ windowMinutes: '60' })
  })

  it('writes an absolute draft as from/to only', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - HOUR, toMs: NOW, windowMinutes: null }
    expect(toRouteQuery(draft)).toEqual({ from: String(NOW - HOUR), to: String(NOW) })
  })

  it('round-trips through the URL', () => {
    const draft = {
      ...emptyDraft(),
      fromMs: NOW - 26 * HOUR,
      toMs: NOW,
      windowMinutes: null,
      instance: 'dns-node-2',
    }
    expect(parseFilterQuery(toRouteQuery(draft))).toEqual(draft)
  })

  it('sends only the params the backend reads', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 2 * HOUR, toMs: NOW, windowMinutes: 60, instance: 'dns-node-2' }
    const search = toRequestQuery(draft, { limit: 200 })
    expect(search).toEqual({
      from: String(NOW - 2 * HOUR),
      to: String(NOW),
      instance: 'dns-node-2',
      limit: '200',
    })
  })
})

describe('rangeLabel', () => {
  it('names a relative hour', () => {
    expect(rangeLabel(relativeDraft(60), NOW)).toBe('近 1 小时')
    expect(rangeLabel(relativeDraft(30), NOW)).toBe('近 30 分钟')
    expect(rangeLabel(relativeDraft(1440), NOW)).toBe('近 24 小时')
  })

  it('names an absolute bracket in local time', () => {
    const draft = { ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW, windowMinutes: null }
    expect(rangeLabel(draft, NOW)).toBe(`${msToDatetime(NOW - 3 * HOUR)} ~ ${msToDatetime(NOW)}`)
  })
})

describe('datetimeToMs', () => {
  it('round-trips through the datetime-local control', () => {
    const text = msToDatetime(NOW)
    expect(datetimeToMs(text)).toBe(NOW - (NOW % 60_000))
  })

  it('rejects blank and malformed input', () => {
    expect(datetimeToMs('')).toBeNull()
    expect(datetimeToMs('not-a-time')).toBeNull()
  })
})

describe('toInvestigationFilter', () => {
  it('carries the resolved bracket plus the echoes', () => {
    const draft = {
      ...emptyDraft(),
      fromMs: NOW - HOUR,
      toMs: NOW,
      windowMinutes: null,
      domain: 'nacos',
      instance: 'dns-node-2',
    }
    expect(toInvestigationFilter(draft, NOW)).toEqual({
      from: NOW - HOUR,
      to: NOW,
      domain: 'nacos',
      responseCode: null,
      instance: 'dns-node-2',
    })
  })
})
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run failureFilter 2>&1 | tail -20`
Expected: FAIL — cannot resolve `../failureFilter`.

- [ ] **Step 3: Write the module**

Create `doggy-dns-ui/src/lib/failureFilter.ts`:

```ts
import type { InvestigationFilter } from '../api/types'

/** Mirrors `MIN_RANGE_MINUTES` in `src/domain.rs`. */
export const MIN_RANGE_MINUTES = 5
/** Mirrors `MAX_RANGE_MINUTES` in `src/domain.rs` — the investigation ceiling. */
export const MAX_RANGE_MINUTES = 7 * 1440
/** Mirrors `FUTURE_TOLERANCE_MS`: the browser clock may run slightly ahead. */
export const FUTURE_TOLERANCE_MS = 60_000
export const DEFAULT_WINDOW_MINUTES = 60
const MINUTE = 60_000

/** What the filter bar holds. Absolute once both `fromMs` and `toMs` are set. */
export interface FilterDraft {
  fromMs: number | null
  toMs: number | null
  windowMinutes: number | null
  domain: string
  responseCode: string
  instance: string
}

export interface Bracket {
  startMs: number
  endMs: number
}

export function emptyDraft(): FilterDraft {
  return {
    fromMs: null,
    toMs: null,
    windowMinutes: null,
    domain: '',
    responseCode: '',
    instance: '',
  }
}

export function relativeDraft(windowMinutes: number): FilterDraft {
  return { ...emptyDraft(), windowMinutes }
}

export function isAbsolute(draft: FilterDraft): boolean {
  return draft.fromMs !== null && draft.toMs !== null
}

type RawQuery = Record<string, string | string[] | null | undefined>

function firstSingle(value: string | string[] | null | undefined): string | null {
  if (value === null || value === undefined) return null
  const text = Array.isArray(value) ? (value[0] ?? null) : value
  return text === '' ? null : text
}

function parseMs(value: string | null): number | null {
  if (value === null) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && Number.isInteger(parsed) ? parsed : null
}

export function parseFilterQuery(query: RawQuery): FilterDraft {
  const minutes = parseMs(firstSingle(query.windowMinutes))
  return {
    fromMs: parseMs(firstSingle(query.from)),
    toMs: parseMs(firstSingle(query.to)),
    windowMinutes: minutes === null ? null : minutes,
    domain: firstSingle(query.domain) ?? '',
    responseCode: firstSingle(query.responseCode) ?? '',
    instance: firstSingle(query.instance) ?? '',
  }
}

/**
 * The URL is the only state source, so the shape it writes matters: relative
 * drafts travel as `windowMinutes` (the server owns "now"), absolute drafts as
 * `from`/`to`. Mixing both would make the URL drift on every refresh.
 */
export function toRouteQuery(draft: FilterDraft): Record<string, string> {
  const query: Record<string, string> = {}
  if (isAbsolute(draft)) {
    query.from = String(draft.fromMs)
    query.to = String(draft.toMs)
  } else if (draft.windowMinutes !== null) {
    query.windowMinutes = String(draft.windowMinutes)
  }
  if (draft.domain) query.domain = draft.domain
  if (draft.responseCode) query.responseCode = draft.responseCode
  if (draft.instance) query.instance = draft.instance
  return query
}

export function resolveBracket(draft: FilterDraft, nowMs: number): Bracket {
  const endMs = draft.toMs ?? nowMs
  if (draft.fromMs !== null && draft.toMs !== null) return { startMs: draft.fromMs, endMs }
  const minutes = draft.windowMinutes ?? DEFAULT_WINDOW_MINUTES
  return { startMs: draft.fromMs ?? endMs - minutes * MINUTE, endMs }
}

/** The same four rules the backend enforces — a mirror, not a second opinion. */
export function validateBracket(bracket: Bracket, nowMs: number): string | null {
  if (bracket.startMs >= bracket.endMs) return '区间起始必须早于结束'
  if (bracket.endMs > nowMs + FUTURE_TOLERANCE_MS) return '区间结束不能超过当前时间 1 分钟'
  const spanMs = bracket.endMs - bracket.startMs
  if (spanMs < MIN_RANGE_MINUTES * MINUTE) return `区间不足 ${MIN_RANGE_MINUTES} 分钟`
  if (spanMs > MAX_RANGE_MINUTES * MINUTE) {
    return `区间超过 ${MAX_RANGE_MINUTES} 分钟（7 天）上限，请收窄`
  }
  return null
}

/** Only the params the endpoint reads; an absolute pair suppresses the shortcut. */
export function toRequestQuery(
  draft: FilterDraft,
  extra: Record<string, string | number> = {},
): Record<string, string> {
  const query: Record<string, string> = {}
  if (isAbsolute(draft)) {
    query.from = String(draft.fromMs)
    query.to = String(draft.toMs)
  } else {
    if (draft.windowMinutes !== null) query.windowMinutes = String(draft.windowMinutes)
    if (draft.toMs !== null) query.to = String(draft.toMs)
  }
  if (draft.domain) query.domain = draft.domain
  if (draft.responseCode) query.responseCode = draft.responseCode
  if (draft.instance) query.instance = draft.instance
  for (const [key, value] of Object.entries(extra)) query[key] = String(value)
  return query
}

export function toInvestigationFilter(draft: FilterDraft, nowMs: number): InvestigationFilter {
  const bracket = resolveBracket(draft, nowMs)
  return {
    from: bracket.startMs,
    to: bracket.endMs,
    domain: draft.domain || null,
    responseCode: draft.responseCode || null,
    instance: draft.instance || null,
  }
}

function pad(value: number): string {
  return String(value).padStart(2, '0')
}

/** `datetime-local` value in the browser's local zone, minute precision. */
export function msToDatetime(ms: number): string {
  const date = new Date(ms)
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function datetimeToMs(value: string): number | null {
  if (!value) return null
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? null : parsed
}

export function rangeLabel(draft: FilterDraft, nowMs: number): string {
  if (isAbsolute(draft)) {
    return `${msToDatetime(draft.fromMs as number)} ~ ${msToDatetime(draft.toMs as number)}`
  }
  const minutes = draft.windowMinutes ?? DEFAULT_WINDOW_MINUTES
  if (minutes < 60) return `近 ${minutes} 分钟`
  if (minutes < 1440) return `近 ${minutes / 60} 小时`
  return `近 ${minutes / 1440} 天`
}
```

`datetimeToMs` truncates to the minute because `msToDatetime` drops seconds, so the round-trip test compares against `NOW - (NOW % 60_000)`; the value stays inside the tolerance the backend allows.

- [ ] **Step 4: Mirror the wire types**

In `doggy-dns-ui/src/api/types.ts`, replace `EvidenceFilter` (`:134-138`) and add the request shape above it:

```ts
/** The bracket and echoes a request carries. Mirrors `FailureQuery`. */
export interface InvestigationFilter {
  from: number
  to: number
  domain: string | null
  responseCode: string | null
  instance: string | null
}

/**
 * Filter echo inside an evidence bundle.
 *
 * Deliberately range-free: the fingerprint is computed over the trace's own
 * evidence, so widening the bracket reuses the stored analysis by design.
 * Mirrors `EvidenceFilter` in `src/evidence/model.rs`.
 */
export interface EvidenceFilter {
  domain: string | null
  responseCode: string | null
  instance: string | null
}
```

`FailureGroup` (`:18-31`) already has `instance: string | null`, so no change there.

- [ ] **Step 5: Run the frontend checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run failureFilter 2>&1 | tail -20`
Expected: PASS.

Run: `cd doggy-dns-ui && bun run test:unit -- --run 2>&1 | tail -30`
Expected: the component specs that still hand `windowMinutes` to the drawer keep failing until Task 10; if a spec asserts the old `EvidenceFilter` shape (`filter: { windowMinutes: 60, … }` at `FailureDrawer.spec.ts:65` and `:185`), update those mock bundles here — the mock is a wire payload, and the type now forbids the old field.

Run: `cd doggy-dns-ui && bun run type-check && bun run lint 2>&1 | tail -20`
Expected: clean except for the files Tasks 9-12 will rewrite; list any remaining error so the next task inherits it knowingly.

- [ ] **Step 6: Commit**

```bash
git add doggy-dns-ui/src/lib doggy-dns-ui/src/api/types.ts doggy-dns-ui/src/components/__tests__
git commit -m "$(cat <<'EOF'
feat: add the frontend filter contract as a pure module

Precedence, the four validation rules, the URL round-trip and the range label
live in one Vue-free module so the failure page and the drawer cannot drift
apart, and so a 7-day bracket is never clamped back to a day.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 9: `useFailureList` follows the draft, not a number

Three behaviours change together (D8, §6.2, and the no-clamp rule): the composable reads a `FilterDraft`, an invalid bracket blocks the request instead of silently querying something else, an absolute bracket stops the 15-second poll, and an instance filter pulls the uncovered count alongside the list.

**Files:**
- Modify: `doggy-dns-ui/src/composables/useFailureList.ts`
- Modify: `doggy-dns-ui/src/views/OverviewView.vue:14-15,89-93`
- Test: `doggy-dns-ui/src/composables/__tests__/useFailureList.spec.ts`

**Interfaces:**
- Consumes: `FilterDraft`, `toRequestQuery`, `validateBracket`, `resolveBracket`, `isAbsolute`, `relativeDraft` (Task 8); `GET /api/failures/uncovered` (Task 4).
- Produces: `useFailureList(draft: Ref<FilterDraft>, limit?: number) -> { groups, markers, error, uncovered, refresh, refreshMarkers }`. Task 11's page and Task 12's overview card consume it.

- [ ] **Step 1: Write the failing composable tests**

`tests` live in `doggy-dns-ui/src/composables/__tests__/useFailureList.spec.ts`; the file already has a `mountList()` helper that mounts a host component with `useFailureList(ref(60))`, a `stubFetch` collecting `{ url, method }`, and unmounts wrappers in `afterEach`. Replace `ref(60)` with `ref(relativeDraft(60))` throughout, then add:

```ts
import { relativeDraft } from '@/lib/failureFilter'

it('sends windowMinutes for a relative draft', async () => {
  const { fetches, wrapper } = mountList(ref(relativeDraft(120)))
  await flushPromises()

  expect(fetches[0].url).toContain('windowMinutes=120')
  expect(fetches[0].url).not.toContain('from=')
  wrapper.unmount()
})

it('sends from/to and stops polling for an absolute draft', async () => {
  vi.useFakeTimers()
  const now = Date.now()
  const draft = ref({
    ...emptyDraft(),
    fromMs: now - 3 * HOUR,
    toMs: now,
    windowMinutes: null,
  })
  const { fetches, timerSet, wrapper } = mountList(draft)
  await flushPromises()

  expect(fetches[0].url).toContain(`from=${now - 3 * HOUR}`)
  expect(fetches[0].url).toContain(`to=${now}`)
  expect(fetches[0].url).not.toContain('windowMinutes=')
  expect(timerSet).toBe(false)

  vi.advanceTimersByTime(60_000)
  await flushPromises()
  expect(fetches.length).toBe(1)
  wrapper.unmount()
  vi.useRealTimers()
})

it('blocks the request and reports the invalid bracket', async () => {
  const draft = ref({ ...emptyDraft(), fromMs: 10, toMs: 20, windowMinutes: null })
  const { fetches, error, wrapper } = mountList(draft)
  await flushPromises()

  expect(fetches).toHaveLength(0)
  expect(error.value).toMatch(/5 分钟/)
  wrapper.unmount()
})

it('refetches when the draft changes and clears the old badges', async () => {
  const draft = ref(relativeDraft(60))
  const { fetches, groups, wrapper } = mountList(draft)
  await flushPromises()
  expect(groups.value.length).toBeGreaterThan(0)

  draft.value = relativeDraft(1440)
  await flushPromises()

  expect(fetches.at(-1)!.url).toContain('windowMinutes=1440')
  wrapper.unmount()
})

it('asks for the uncovered count only when an instance is selected', async () => {
  const draft = ref({ ...relativeDraft(60), instance: 'dns-node-2' })
  const { fetches, uncovered, wrapper } = mountList(draft)
  await flushPromises()

  expect(fetches.some((entry) => entry.url.startsWith('/api/failures/uncovered?'))).toBe(true)
  expect(uncovered.value).toBe(1)

  fetches.length = 0
  draft.value = relativeDraft(60)
  await flushPromises()
  expect(fetches.some((entry) => entry.url.startsWith('/api/failures/uncovered?'))).toBe(false)
  expect(uncovered.value).toBe(0)
  wrapper.unmount()
})
```

Extend `mountList` to return `{ fetches, groups, uncovered, error, timerSet, wrapper }`: spy on `setInterval` (or read a module-level flag the test sets) so `timerSet` reports whether a poll was scheduled, and make `stubFetch` answer `/api/failures` with two groups and `/api/failures/uncovered` with `{ unlabeled: 1 }`. Import `emptyDraft` and add `const HOUR = 3_600_000` at the top of the file.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run useFailureList 2>&1 | tail -30`
Expected: FAIL — type error on the `Ref<FilterDraft>` argument, then the URL and polling assertions.

- [ ] **Step 3: Rewrite the composable**

Replace `doggy-dns-ui/src/composables/useFailureList.ts` in full:

```ts
import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { AnalysisMarker, FailureGroup } from '@/api/types'
import {
  isAbsolute,
  resolveBracket,
  toRequestQuery,
  validateBracket,
  type FilterDraft,
} from '@/lib/failureFilter'

/** The markers endpoint accepts at most 200 unique trace ids per request. */
const MAX_MARKER_TRACE_IDS = 200
const POLL_INTERVAL_MS = 15_000

function buildQuery(params: Record<string, string>): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) search.set(key, value)
  return search.toString()
}

export function useFailureList(draft: Ref<FilterDraft>, limit = 5) {
  const groups = ref<FailureGroup[]>([])
  const markers = ref<Set<string>>(new Set())
  const uncovered = ref(0)
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let markerSeq = 0

  async function refreshMarkers() {
    const seq = ++markerSeq
    const ids = Array.from(
      new Set(
        groups.value
          .map((group) => group.traceId)
          .filter((traceId): traceId is string => traceId !== null),
      ),
    ).slice(0, MAX_MARKER_TRACE_IDS)

    if (ids.length === 0) {
      markers.value = new Set()
      return
    }

    const query = ids.map((traceId) => encodeURIComponent(traceId)).join(',')
    try {
      const found = await apiGet<AnalysisMarker[]>(`/api/analysis/markers?traceIds=${query}`)
      // A newer draft's refresh owns the markers; drop this older answer.
      if (seq !== markerSeq) return
      const requested = new Set(ids)
      markers.value = new Set(
        found.map((marker) => marker.traceId).filter((traceId) => requested.has(traceId)),
      )
    } catch {
      // Markers are cosmetic: never surface an error or hide the failure list.
      if (seq !== markerSeq) return
      markers.value = new Set()
    }
  }

  async function refreshUncovered() {
    if (!draft.value.instance) {
      uncovered.value = 0
      return
    }
    try {
      const found = await apiGet<{ unlabeled: number }>(
        `/api/failures/uncovered?${buildQuery(toRequestQuery(draft.value))}`,
      )
      uncovered.value = found.unlabeled
    } catch {
      // The notice is advisory; a failed count must not hide the list.
      uncovered.value = 0
    }
  }

  function restartTimer() {
    if (timer) {
      clearInterval(timer)
      timer = null
    }
    // An absolute bracket is a fixed question, so refreshing it on a timer would
    // only move the answer under the operator's feet.
    if (!isAbsolute(draft.value)) timer = setInterval(() => void refresh(), POLL_INTERVAL_MS)
  }

  async function refresh() {
    const bracket = resolveBracket(draft.value, Date.now())
    const invalid = validateBracket(bracket, Date.now())
    if (invalid) {
      error.value = invalid
      groups.value = []
      markers.value = new Set()
      uncovered.value = 0
      return
    }

    try {
      groups.value = await apiGet<FailureGroup[]>(
        `/api/failures?${buildQuery({ ...toRequestQuery(draft.value), limit: String(limit) })}`,
      )
      error.value = null
    } catch (cause) {
      error.value = String(cause)
      // A failed switch must not leave the previous bracket's badges behind.
      markers.value = new Set()
      uncovered.value = 0
      return
    }
    await Promise.all([refreshMarkers(), refreshUncovered()])
  }

  onMounted(() => {
    void refresh()
    restartTimer()
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(draft, () => {
    void refresh()
    restartTimer()
  })

  return { groups, markers, error, uncovered, refresh, refreshMarkers }
}
```

The `watch(draft, …)` fires on identity change, which is what the page wants: it assigns a freshly parsed draft on every URL write rather than mutating fields in place.

- [ ] **Step 4: Adapt the overview card**

`doggy-dns-ui/src/views/OverviewView.vue:14-15` becomes:

```ts
const draft = computed(() => relativeDraft(windowMinutes.value))
const { groups, markers, error } = useFailureList(draft, 5)
```

Leave the existing `windowMinutes` ref/computed and its 1h/6h/24h buttons exactly as they are — the overview is the live class and keeps clamping through the relative shortcut. Do not destructure `uncovered` here: the card has no instance filter, so the count is always 0 and an unused binding fails `bun run lint`. Add `import { relativeDraft } from '@/lib/failureFilter'`.

- [ ] **Step 5: Run the frontend checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run useFailureList OverviewView 2>&1 | tail -30`
Expected: PASS. `OverviewView.spec.ts` mounts with `createPinia()` and `stubs: { TrendChart: true }` and no router; Task 12 adds the router, so leave it router-free here.

Run: `cd doggy-dns-ui && bun run type-check 2>&1 | tail -20`
Expected: clean for these two files; `FailureDrawer.vue` and `useInvestigation.ts` still take `windowMinutes` and are handled next.

- [ ] **Step 6: Commit**

```bash
git add doggy-dns-ui/src/composables doggy-dns-ui/src/views/OverviewView.vue
git commit -m "$(cat <<'EOF'
feat: drive the failure list from a filter draft

The composable now reads from/to/instance, refuses to query an invalid bracket
instead of guessing, stops polling once the bracket is fixed, and reports how
many in-scope failures the host filter cannot reach.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 10: The investigation and the drawer take a filter, not a number

The drawer is the only consumer of `useInvestigation`, and its handoff context is what makes the copilot panel's "回到调查页" link keep the bracket (§6.5).

**Files:**
- Modify: `doggy-dns-ui/src/composables/useInvestigation.ts:48,117,172`
- Modify: `doggy-dns-ui/src/components/FailureDrawer.vue:11,17-21,205-207,225-237,324-335`
- Modify: `doggy-dns-ui/src/stores/ui.ts:8-13` (`HandoffContext`)
- Modify: `doggy-dns-ui/src/components/AiPanel.vue:52-60`
- Test: `doggy-dns-ui/src/components/__tests__/FailureDrawer.spec.ts`, `AiPanel.spec.ts`, `composables/__tests__/useInvestigation.spec.ts` (if present)

**Interfaces:**
- Consumes: `InvestigationFilter` (Task 8), `rangeLabel` (Task 8), the range-free `EvidenceFilter` echo (Task 5).
- Produces: `useInvestigation(traceId: Ref<string | null>, filter: Ref<InvestigationFilter>)`; `FailureDrawer.vue` props `{ traceId: string; filter: InvestigationFilter; analysisId?: string }`; `HandoffContext { traceId, verdict, summary, fromMs?, toMs? }`. Task 11 passes the page's resolved filter; Task 12 makes the deep-link route the only way to open the drawer.

- [ ] **Step 1: Write the failing spec updates**

In `doggy-dns-ui/src/components/__tests__/FailureDrawer.spec.ts`, replace the mount helper's props (`:214`) and the two mock bundle echoes (`:65`, `:185`):

```ts
const filter = ref<InvestigationFilter>({
  from: 1_761_433_200_000,
  to: 1_761_436_800_000,
  domain: null,
  responseCode: null,
  instance: null,
})

function mountDrawer(props: Partial<{ traceId: string; analysisId: string }> = {}) {
  return mount(FailureDrawer, {
    props: { traceId: 't1', filter: filter.value, ...props },
    global: { plugins: [createPinia()] },
  })
}
```

Add the pins:

```ts
it('requests evidence for the bracket it was given', async () => {
  mountDrawer()
  await flushPromises()

  const url = fetchCalls.find((call) => call.url.includes('/evidence'))?.url ?? ''
  expect(url).toContain('from=1761433200000')
  expect(url).toContain('to=1761436800000')
  expect(url).not.toContain('windowMinutes=')
})

it('starts the analysis for the same bracket', async () => {
  const wrapper = mountDrawer()
  await flushPromises()

  await wrapper.find('[data-testid="investigate-button"]').trigger('click')
  await flushPromises()

  const url = fetchCalls.find((call) => call.method === 'POST')?.url ?? ''
  expect(url).toContain('from=1761433200000')
  expect(url).toContain('to=1761436800000')
})

it('shows the bracket in the header chips', async () => {
  const wrapper = mountDrawer()
  await flushPromises()

  expect(wrapper.find('[data-testid="range-chip"]').text()).toBe(
    '2025-10-26T05:40 ~ 2025-10-26T06:40',
  )
})

it('hands off the bracket rather than the window', async () => {
  const wrapper = mountDrawer()
  await flushPromises()

  await wrapper.find('[data-testid="handoff-button"]').trigger('click')
  await flushPromises()

  const ui = useUiStore()
  expect(ui.handoff?.fromMs).toBe(1_761_433_200_000)
  expect(ui.handoff?.toMs).toBe(1_761_436_800_000)
  expect(ui.handoff).not.toHaveProperty('windowMinutes')
})

it('re-loads when only the bracket changes', async () => {
  const wrapper = mountDrawer()
  await flushPromises()
  fetchCalls.length = 0

  await wrapper.setProps({
    filter: { ...filter.value, from: 1_761_429_600_000 },
  })
  await flushPromises()

  expect(fetchCalls.some((call) => call.url.includes('from=1761429600000'))).toBe(true)
})
```

The header-chip expectation is the one test that must be computed once, not derived: run `node -e "console.log(new Date(1761433200000).toString())"` and, if the CI machine is not in the same zone as the developer, build the expectation with the exported `msToDatetime` helper instead of a literal —

```ts
expect(wrapper.find('[data-testid="range-chip"]').text()).toBe(
  `${msToDatetime(1_761_433_200_000)} ~ ${msToDatetime(1_761_436_800_000)}`,
)
```

In `AiPanel.spec.ts:84-99`, the handoff fixture loses `windowMinutes: 30` and gains `fromMs`/`toMs`, and the expected push query follows.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailureDrawer AiPanel 2>&1 | tail -40`
Expected: FAIL — `filter` is an unknown prop, so no bracket reaches the URL.

- [ ] **Step 3: Make the composable bracket-aware**

`doggy-dns-ui/src/composables/useInvestigation.ts:48` and the two request builders:

```ts
export function useInvestigation(
  traceId: Ref<string | null>,
  filter: Ref<InvestigationFilter>,
) {
```

```ts
const bundle = await apiGet<EvidenceBundle>(
  `/api/failures/${encodeURIComponent(trace)}/evidence?${filterQuery(filter.value)}`,
)
```

```ts
const params = new URLSearchParams(filterQuery(filter.value))
if (force) params.set('force', 'true')
```

Add the helper above `useInvestigation` (module-private, next to `snapshotState`):

```ts
function filterQuery(filter: InvestigationFilter): string {
  const params = new URLSearchParams()
  params.set('from', String(filter.from))
  params.set('to', String(filter.to))
  if (filter.domain) params.set('domain', filter.domain)
  if (filter.responseCode) params.set('responseCode', filter.responseCode)
  if (filter.instance) params.set('instance', filter.instance)
  return params.toString()
}
```

Import `type InvestigationFilter` from `@/api/types`. The `stale` computed (`:54-59`) does not change — the server owns the fingerprint, and a widened bracket is intentionally not stale (D11), so no client-side range comparison belongs there.

- [ ] **Step 4: Change the drawer props, header, watch and handoff**

`doggy-dns-ui/src/components/FailureDrawer.vue:11,17-21`:

```ts
const props = defineProps<{
  traceId: string
  filter: InvestigationFilter
  analysisId?: string
}>()

const trace = toRef(props, 'traceId')
const filter = toRef(props, 'filter')
const investigation = useInvestigation(trace, filter)
```

`FailureDrawer.vue:205-207` re-keys on the serialized filter, because `toRef` on an object prop changes identity but the fields are what matter:

```ts
watch(
  [trace, () => JSON.stringify(props.filter), () => props.analysisId],
  () => void investigation.loadEvidence(),
)
```

Add the range chip to the header row (`FailureDrawer.vue:324-335`, beside the `responseCode`/`×count`/`instance` chips):

```html
<span
  data-testid="range-chip"
  class="rounded-full bg-slate-800 px-2 py-0.5 text-xs text-slate-300"
  >{{ bracketLabel }}</span
>
```

with

```ts
const bracketLabel = computed(() =>
  `${msToDatetime(props.filter.from)} ~ ${msToDatetime(props.filter.to)}`,
)
```

and `import { msToDatetime } from '@/lib/failureFilter'`.

The handoff (`FailureDrawer.vue:225-237`) and the prompt text (`:138-149`) switch from the window to the bracket:

```ts
ui.rememberHandoff({
  traceId: props.traceId,
  verdict: result.value.verdict.category,
  summary: result.value.verdict.summary,
  fromMs: props.filter.from,
  toMs: props.filter.to,
})
```

```ts
const lines = [`请继续调查这组 DNS 失败：trace_id=${props.traceId}`]
lines.push(`区间=${msToDatetime(props.filter.from)} ~ ${msToDatetime(props.filter.to)}`)
```

- [ ] **Step 5: Move the handoff type and the panel link**

`doggy-dns-ui/src/stores/ui.ts:8-13`:

```ts
export interface HandoffContext {
  traceId: string
  verdict: string
  summary: string
  /** Set together when the investigation was framed absolutely. */
  fromMs?: number
  toMs?: number
}
```

`doggy-dns-ui/src/components/AiPanel.vue:52-60`:

```ts
function reopenFailure() {
  if (!handoff.value) return
  const query: Record<string, string> = {}
  if (handoff.value.fromMs !== undefined && handoff.value.toMs !== undefined) {
    query.from = String(handoff.value.fromMs)
    query.to = String(handoff.value.toMs)
  }
  router.push({ name: 'failure-detail', params: { traceId: handoff.value.traceId }, query })
}
```

The route name is `failure-detail` as of Task 12; until then keep pushing `/failures/{traceId}` with the same query, and land both in Task 12's commit if that reads cleaner.

- [ ] **Step 6: Run the checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailureDrawer AiPanel useInvestigation 2>&1 | tail -30`
Expected: PASS.

Run: `cd doggy-dns-ui && bun run type-check 2>&1 | tail -20`
Expected: only `OverviewView.vue`'s `<FailureDrawer>` mount and `FailureView.vue` still pass `:window-minutes`; Task 12 deletes both.

- [ ] **Step 7: Commit**

```bash
git add doggy-dns-ui/src/composables/useInvestigation.ts doggy-dns-ui/src/components/FailureDrawer.vue doggy-dns-ui/src/components/AiPanel.vue doggy-dns-ui/src/stores/ui.ts doggy-dns-ui/src/components/__tests__
git commit -m "$(cat <<'EOF'
feat: give the drawer and its handoff the absolute bracket

Evidence and analysis requests now carry from/to/domain/responseCode/instance,
the header states which bracket was asked about, and the copilot handoff keeps
the interval instead of a window that no longer describes it.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 11: The deep link resolves a bracket, and the clamp dies

D4's frontend half: `FailureView.vue:13-14,27-35` silently folded anything above 1440 minutes back to a day, which is precisely the promise the 7-day look-back breaks. The route now parses its own query and owns one `InvestigationFilter`.

**Files:**
- Modify: `doggy-dns-ui/src/views/FailureView.vue` (full rewrite of the script block)
- Test: `doggy-dns-ui/src/views/__tests__/FailureView.spec.ts`

**Interfaces:**
- Consumes: `parseFilterQuery`, `resolveBracket`, `validateBracket`, `isAbsolute`, `toInvestigationFilter` (Task 8); the new drawer props (Task 10).
- Produces: `FailureView` reading `?from&to&windowMinutes&domain&responseCode&instance&analysis` and passing `filter: InvestigationFilter` down; a named route `failure-detail` (already `/failures/:traceId`, `src/router/index.ts`) that Task 12 re-parents. Task 12's list links to it.

- [ ] **Step 1: Replace the spec's clamp table with bracket tests**

`FailureView.spec.ts` mounts with a `DrawerStub` and asserts the whole clamp table (`:65-91`) plus close→overview (`:102-111`). Delete the clamp rows and write:

```ts
const NOW = Date.now()
const HOUR = 3_600_000

function mountView(query: Record<string, string>, params = { traceId: 't1' }) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', name: 'overview', component: true },
      { path: '/failures/:traceId', name: 'failure-detail', component: FailureView },
    ],
  })
  return mount(FailureView, {
    props: {},
    global: {
      plugins: [router, createPinia()],
      stubs: { FailureDrawer: DrawerStub },
      mocks: { $route: { query, params } },
    },
  })
}
```

Use the file's existing router-mocking technique rather than inventing a second one — if it already pushes the router to a URL and awaits `router.isReady()`, keep that shape and only change the query strings. The assertions:

```ts
it('passes an absolute bracket straight to the drawer', async () => {
  const from = NOW - 26 * HOUR
  const wrapper = await mountViewWithQuery({ from: String(from), to: String(NOW) })

  const passed = wrapper.findComponent(DrawerStub).props('filter')
  expect(passed.from).toBe(from)
  expect(passed.to).toBe(NOW)
  expect(passed.instance).toBeNull()
})

it('expands a relative shortcut against the server clock without clamping', async () => {
  const wrapper = await mountViewWithQuery({ windowMinutes: '10080' })

  const passed = wrapper.findComponent(DrawerStub).props('filter')
  expect(passed.to - passed.from).toBe(10080 * 60_000)
})

it('keeps a seven-day bracket instead of folding it back to a day', async () => {
  const from = NOW - 7 * 24 * HOUR
  const wrapper = await mountViewWithQuery({ from: String(from), to: String(NOW) })

  expect(wrapper.findComponent(DrawerStub).props('filter').from).toBe(from)
  expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(false)
})

it('carries domain, responseCode and instance into the request filter', async () => {
  const wrapper = await mountViewWithQuery({
    from: String(NOW - 2 * HOUR),
    to: String(NOW),
    domain: 'nacos',
    responseCode: 'SERVFAIL',
    instance: 'dns-node-2',
  })

  const passed = wrapper.findComponent(DrawerStub).props('filter')
  expect(passed).toMatchObject({ domain: 'nacos', responseCode: 'SERVFAIL', instance: 'dns-node-2' })
})

it('names an impossible bracket instead of querying something else', async () => {
  const wrapper = await mountViewWithQuery({ from: String(NOW), to: String(NOW) })

  expect(wrapper.find('[data-testid="invalid-range"]').text()).toMatch(/早于|结束|分钟/)
  expect(wrapper.findComponent(DrawerStub).exists()).toBe(false)
})

it('still restores a deep-linked analysis id', async () => {
  const from = NOW - 2 * HOUR
  const wrapper = await mountViewWithQuery({ from: String(from), to: String(NOW), analysis: 'inv-9' })

  expect(wrapper.findComponent(DrawerStub).props('analysisId')).toBe('inv-9')
})
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailureView 2>&1 | tail -30`
Expected: FAIL — the view still emits `window-minutes` and clamps at 1440.

- [ ] **Step 3: Rewrite the view**

Replace the `<script setup lang="ts">` block of `doggy-dns-ui/src/views/FailureView.vue` (and the `:window-minutes` binding in its template) with:

```ts
<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import FailureDrawer from '@/components/FailureDrawer.vue'
import {
  parseFilterQuery,
  resolveBracket,
  toInvestigationFilter,
  validateBracket,
} from '@/lib/failureFilter'

const route = useRoute()
const router = useRouter()

const draft = computed(() => parseFilterQuery(route.query))
const bracket = computed(() => resolveBracket(draft.value, Date.now()))
const invalid = computed(() => validateBracket(bracket.value, Date.now()))
const filter = computed(() => toInvestigationFilter(draft.value, Date.now()))
const traceId = computed(() => String(route.params.traceId ?? ''))
const analysisId = computed(() => {
  const value = route.query.analysis
  return typeof value === 'string' && value !== '' ? value : undefined
})

function close() {
  // Returning to the overview keeps working until the failure list owns the
  // drawer mount; Task 12 replaces this with a query-preserving push.
  void router.push({ name: 'overview' })
}
</script>

<template>
  <div class="mx-auto flex w-full max-w-6xl flex-col gap-4 px-6 py-6">
    <p
      v-if="invalid"
      data-testid="invalid-range"
      class="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error"
    >
      {{ invalid }}
    </p>
    <FailureDrawer
      v-else-if="traceId"
      :trace-id="traceId"
      :filter="filter"
      :analysis-id="analysisId"
      @close="close"
    />
  </div>
</template>
```

`Date.now()` is called per computed evaluation, so a relative deep link (`?windowMinutes=60`) resolves against the browser clock at mount time; that is the same instant the operator clicked, and the bracket never round-trips into the URL (deviation 5).

- [ ] **Step 4: Run the checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailureView 2>&1 | tail -30`
Expected: PASS.

Run: `cd doggy-dns-ui && bun run type-check && bun run build 2>&1 | tail -20`
Expected: clean. `OverviewView.vue` still mounts its own `<FailureDrawer v-if="ui.drawerTraceId" :window-minutes>` — that is Task 12's deletion, and it is the one remaining `windowMinutes` reference the type-checker will flag. If it does not fail here (the drawer's prop is now required and `:window-minutes` is unknown), delete that block and its `openFailure` call site in this task instead, and skip ahead to Task 12's route work; do not leave a knowingly broken template between commits.

- [ ] **Step 5: Commit**

```bash
git add doggy-dns-ui/src/views/FailureView.vue doggy-dns-ui/src/views/__tests__/FailureView.spec.ts
git commit -m "$(cat <<'EOF'
fix: stop clamping the failure deep link back to a day

The route resolves its own bracket, so a 7-day link stays 7 days and an
impossible link is named out loud instead of quietly rewritten.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 12: The `/failures` page — filter bar, full-field rows, and the drawer's one mount

Spec §5 方案 1 plus the nesting that makes D5 true: the list is the parent route and the drawer is its child, so the URL is the only way in and the list stays mounted underneath it.

**Files:**
- Create: `doggy-dns-ui/src/components/FailureFilterBar.vue`
- Create: `doggy-dns-ui/src/views/FailuresView.vue`
- Modify: `doggy-dns-ui/src/router/index.ts:19-23` (parent `failures` + child `failure-detail`)
- Modify: `doggy-dns-ui/src/components/SideRail.vue` (nav link)
- Modify: `doggy-dns-ui/src/views/FailureView.vue` (page chrome moves to the parent)
- Test: `doggy-dns-ui/src/router/__tests__/index.spec.ts`, `doggy-dns-ui/src/views/__tests__/FailuresView.spec.ts`

**Interfaces:**
- Consumes: `useFailureList(draft, limit)` + `uncovered` (Task 9), `FilterDraft`/`toRouteQuery`/`parseFilterQuery`/`rangeLabel`/`validateBracket`/`datetimeToMs`/`msToDatetime`/`MIN_RANGE_MINUTES`/`MAX_RANGE_MINUTES` (Task 8), `useInstances()` (existing), the `failure-detail` route name used by `AiPanel.reopenFailure` (Task 10).
- Produces: `/failures?from&to&windowMinutes&domain&responseCode&instance&limit` and `/failures/:traceId?…&analysis=<id>`; `FailureFilterBar` with `v-model:draft`; `FailuresView` rendering `<RouterView/>` for the drawer. Task 13 links the overview card here.

- [ ] **Step 1: Write the failing route and page tests**

Replace `doggy-dns-ui/src/router/__tests__/index.spec.ts:10-15` (which asserted `query.windowMinutes` on the old `failure` route) with:

```ts
it('nests the detail route under the failure list', () => {
  const list = routes.find((route) => route.name === 'failures')
  const detail = routes.find((route) => route.name === 'failure-detail')

  expect(list?.path).toBe('/failures')
  expect(list?.children?.map((child) => child.path)).toContain(':traceId')
  expect(detail?.path).toBe(':traceId')
})

it('keeps the bracket in the detail link', () => {
  const detail = routes.find((route) => route.name === 'failure-detail')
  expect(detail).toBeDefined()
})
```

Create `doggy-dns-ui/src/views/__tests__/FailuresView.spec.ts`:

```ts
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'

import FailuresView from '../FailuresView.vue'

const fetches: string[] = []

const ONE_GROUP = {
  key: 'trace:4bf9',
  traceId: '4bf92f3577b34da6',
  query: 'api.internal.',
  qtype: 'A',
  instance: 'dns-node-2',
  authority: 'forward[8.8.8.8:53]',
  responseCode: 'SERVFAIL',
  error: 'upstream timed out',
  count: 12,
  firstSeen: 1_761_433_200_000,
  lastSeen: 1_761_436_800_000,
  durationMs: 5012,
}

function makeRouter(initial: string) {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: '/failures',
        component: FailuresView,
        children: [{ path: ':traceId', name: 'failure-detail', component: true }],
      },
    ],
  })
}

async function mountView(initial: string) {
  const router = makeRouter(initial)
  router.push(initial)
  await router.isReady()
  const wrapper = mount(FailuresView, { global: { plugins: [router, createPinia()] } })
  await flushPromises()
  return { router, wrapper }
}

beforeEach(() => {
  fetches.length = 0
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = String(input)
    fetches.push(url)
    if (url.includes('/api/instances')) {
      return new Response(JSON.stringify([{ id: 'dns-node-1', host: 'dns-node-1' }]), {
        headers: { 'content-type': 'application/json' },
      })
    }
    if (url.includes('/api/failures/uncovered')) {
      return new Response(JSON.stringify({ unlabeled: 2 }), {
        headers: { 'content-type': 'application/json' },
      })
    }
    return new Response(JSON.stringify([ONE_GROUP]), {
      headers: { 'content-type': 'application/json' },
    })
  })
})

it('writes the bracket into the URL when the filter changes', async () => {
  const { router, wrapper } = await mountView('/failures')

  await wrapper.find('[data-testid="quick-7d"]').trigger('click')
  await flushPromises()

  expect(router.currentRoute.value.query.windowMinutes).toBe('10080')
  expect(fetches.some((url) => url.includes('windowMinutes=10080'))).toBe(true)
})

it('reads the bracket from the URL on mount', async () => {
  const from = 1_761_429_600_000
  const to = 1_761_436_800_000
  await mountView(`/failures?from=${from}&to=${to}&instance=dns-node-2`)

  expect(fetches.some((url) => url.includes(`from=${from}`))).toBe(true)
  expect(fetches.some((url) => url.includes('instance=dns-node-2'))).toBe(true)
})

it('names an invalid bracket and does not fetch the list', async () => {
  const { wrapper } = await mountView('/failures?from=10&to=20')

  expect(wrapper.find('[data-testid="invalid-range"]').exists()).toBe(true)
  expect(fetches.some((url) => url.includes('/api/failures?'))).toBe(false)
})

it('shows the uncovered notice with a way to clear the host filter', async () => {
  const { wrapper } = await mountView('/failures?windowMinutes=10080&instance=dns-node-2')

  const notice = wrapper.find('[data-testid="uncovered-notice"]')
  expect(notice.exists()).toBe(true)
  expect(notice.text()).toContain('2')
  expect(notice.text()).toContain('没有主机标签')

  await notice.find('button').trigger('click')
  await flushPromises()
  expect(wrapper.find('[data-testid="uncovered-notice"]').exists()).toBe(false)
})

it('warns when the 200-group cap is hit', async () => {
  // The page always asks for 200 rows, so the cap is detected from the length of
  // the answer, not from the request.
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/instances') || url.includes('/api/analysis/markers')) {
      return new Response('[]', { headers: { 'content-type': 'application/json' } })
    }
    const rows = Array.from({ length: 200 }, (_unused, index) => ({
      ...ONE_GROUP,
      key: `trace:t${index}`,
      traceId: `t${index}`,
    }))
    return new Response(JSON.stringify(rows), { headers: { 'content-type': 'application/json' } })
  })

  const { wrapper } = await mountView('/failures?windowMinutes=10080')

  expect(wrapper.find('[data-testid="cap-notice"]').text()).toContain('仅显示前')
})

it('stops auto-refreshing for an absolute bracket', async () => {
  vi.useFakeTimers()
  const now = Date.now()
  await mountView(`/failures?from=${now - 3 * 3_600_000}&to=${now}`)
  fetches.length = 0
  vi.advanceTimersByTime(60_000)
  await flushPromises()

  expect(fetches).toHaveLength(0)
  vi.useRealTimers()
})

it('links a row to the detail route with the bracket preserved', async () => {
  const { router, wrapper } = await mountView('/failures?windowMinutes=10080')

  await wrapper.find('[data-testid="failure-row"]').trigger('click')
  await flushPromises()

  expect(router.currentRoute.value.name).toBe('failure-detail')
  expect(router.currentRoute.value.query.windowMinutes).toBe('10080')
})
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailuresView index 2>&1 | tail -30`
Expected: FAIL — `../FailuresView.vue` does not exist.

- [ ] **Step 3: Build the filter bar**

Create `doggy-dns-ui/src/components/FailureFilterBar.vue`:

```vue
<script setup lang="ts">
import { computed } from 'vue'

import {
  MAX_RANGE_MINUTES,
  MIN_RANGE_MINUTES,
  datetimeToMs,
  emptyDraft,
  msToDatetime,
  relativeDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

const props = defineProps<{
  modelValue: FilterDraft
  invalid: string | null
  instances: string[]
}>()

const emit = defineEmits<{ 'update:modelValue': [draft: FilterDraft] }>()

const RESPONSE_CODES = ['SERVFAIL', 'REFUSED', 'FORMERR', 'NOERROR']
const QUICK_WINDOWS = [
  { label: '1 小时', minutes: 60, testId: 'quick-1h' },
  { label: '24 小时', minutes: 1440, testId: 'quick-24h' },
  { label: '7 天', minutes: MAX_RANGE_MINUTES, testId: 'quick-7d' },
]

function patch(next: Partial<FilterDraft>) {
  emit('update:modelValue', { ...props.modelValue, ...next })
}

const fromText = computed(() =>
  props.modelValue.fromMs === null ? '' : msToDatetime(props.modelValue.fromMs),
)
const toText = computed(() =>
  props.modelValue.toMs === null ? '' : msToDatetime(props.modelValue.toMs),
)

function setFrom(value: string) {
  patch({ fromMs: datetimeToMs(value), toMs: props.modelValue.toMs, windowMinutes: null })
}

function setTo(value: string) {
  patch({ toMs: datetimeToMs(value), windowMinutes: null })
}

function useRelative(minutes: number) {
  emit('update:modelValue', { ...relativeDraft(minutes), domain: props.modelValue.domain,
    responseCode: props.modelValue.responseCode, instance: props.modelValue.instance })
}

function resetTime() {
  emit('update:modelValue', { ...emptyDraft(), ...props.modelValue, fromMs: null, toMs: null,
    windowMinutes: 60 })
}

const activeQuick = computed(() =>
  !props.modelValue.fromMs && !props.modelValue.toMs ? props.modelValue.windowMinutes : null,
)
</script>

<template>
  <section class="panel flex flex-wrap items-end gap-3 px-4 py-3">
    <div class="flex gap-2">
      <button
        v-for="preset in QUICK_WINDOWS"
        :key="preset.minutes"
        :data-testid="preset.testId"
        class="focus-ring rounded-lg px-2.5 py-1 text-xs"
        :class="
          activeQuick === preset.minutes
            ? 'bg-accent/15 text-accent'
            : 'bg-surface-2 text-muted hover:text-text'
        "
        @click="useRelative(preset.minutes)"
      >
        {{ preset.label }}
      </button>
    </div>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      起
      <input
        type="datetime-local"
        :value="fromText"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="setFrom(($event.target as HTMLInputElement).value)"
      />
    </label>
    <label class="flex flex-col gap-1 text-[11px] text-muted">
      止
      <input
        type="datetime-local"
        :value="toText"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="setTo(($event.target as HTMLInputElement).value)"
      />
    </label>
    <button
      data-testid="reset-time"
      class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-muted hover:text-text"
      @click="resetTime"
    >
      回到近 1 小时
    </button>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      实例
      <select
        data-testid="instance-select"
        :value="modelValue.instance"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ instance: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部主机</option>
        <option v-for="host in instances" :key="host" :value="host">{{ host }}</option>
      </select>
    </label>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      响应码
      <select
        :value="modelValue.responseCode"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ responseCode: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部</option>
        <option v-for="code in RESPONSE_CODES" :key="code" :value="code">{{ code }}</option>
      </select>
    </label>

    <label class="flex min-w-40 flex-1 flex-col gap-1 text-[11px] text-muted">
      域名包含
      <input
        :value="modelValue.domain"
        placeholder="nacos.local"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @input="patch({ domain: ($event.target as HTMLInputElement).value })"
      />
    </label>

    <p v-if="invalid" data-testid="invalid-range" class="w-full text-xs text-error">
      {{ invalid }}
    </p>
    <p v-else class="w-full text-[11px] text-muted">
      回溯链支持 5 分钟到 {{ MAX_RANGE_MINUTES }} 分钟（7 天）；看板与实例视图仍限 1440 分钟。
    </p>
  </section>
</template>
```

`MIN_RANGE_MINUTES` is imported for the hint line; if the final copy does not use it, drop the import rather than leaving an unused binding for `bun run lint`. `resetTime` is the escape hatch out of an absolute bracket, which is what the "uncovered" action also needs.

- [ ] **Step 4: Build the page**

Create `doggy-dns-ui/src/views/FailuresView.vue`:

```vue
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'

import { useFailureList } from '@/composables/useFailureList'
import { useInstances } from '@/composables/useInstances'
import FailureList from '@/components/FailureList.vue'
import FailureFilterBar from '@/components/FailureFilterBar.vue'
import {
  parseFilterQuery,
  resolveBracket,
  toRouteQuery,
  validateBracket,
  type FilterDraft,
} from '@/lib/failureFilter'

const PAGE_LIMIT = 200
const route = useRoute()
const router = useRouter()

// The URL is the only state source (§6.5): the draft is derived, writes go back
// to the router, and the two watchers compare serialized forms so they cannot
// ping-pong each other into a loop.
const draft = ref<FilterDraft>(parseFilterQuery(route.query))
const bracket = computed(() => resolveBracket(draft.value, Date.now()))
const invalid = computed(() => validateBracket(bracket.value, Date.now()))
const { groups, markers, error, uncovered } = useFailureList(draft, PAGE_LIMIT)

// The host options come from the instance view, not from the rows currently on
// screen: an operator narrowing to a quiet node must still be able to pick it.
const { instances } = useInstances()
const hosts = computed(() =>
  instances.value.map((instance) => instance.host).filter((host) => host !== ''),
)

const capped = computed(() => groups.value.length >= PAGE_LIMIT)

function onSelect(traceId: string) {
  void router.push({
    name: 'failure-detail',
    params: { traceId },
    query: toRouteQuery(draft.value),
  })
}

watch(
  () => JSON.stringify(toRouteQuery(draft.value)),
  (serialized) => {
    const current = JSON.stringify(toRouteQuery(parseFilterQuery(route.query)))
    if (serialized === current) return
    void router.replace({ name: 'failures-list', query: JSON.parse(serialized) })
  },
)

watch(
  () => route.fullQuery,
  (query) => {
    const incoming = toRouteQuery(parseFilterQuery(route.query))
    if (JSON.stringify(incoming) === JSON.stringify(toRouteQuery(draft.value))) return
    draft.value = parseFilterQuery({ ...query })
  },
)
</script>

<template>
  <div class="flex flex-col gap-4 px-6 py-6">
    <FailureFilterBar v-model="draft" :invalid="invalid" :instances="hosts" />

    <p
      v-if="invalid"
      data-testid="invalid-range"
      class="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error"
    >
      {{ invalid }}
    </p>
    <p v-else-if="error" class="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-sm">
      {{ error }}
    </p>

    <p
      v-if="draft.instance && uncovered > 0 && !invalid"
      data-testid="uncovered-notice"
      class="flex flex-wrap items-center gap-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-sm"
    >
      有 {{ uncovered }} 条遥测没有主机标签，未被此过滤覆盖。
      <button
        class="focus-ring rounded-md bg-surface-2 px-2 py-0.5 text-xs text-accent"
        @click="draft = { ...draft, fromMs: null, toMs: null, windowMinutes: 60, instance: '' }"
      >
        查看全部
      </button>
    </p>

    <p
      v-if="capped"
      data-testid="cap-notice"
      class="text-xs text-muted"
    >
      仅显示前 {{ PAGE_LIMIT }} 组，请收窄时间或筛选。
    </p>

    <FailureList
      :groups="groups"
      :markers="markers"
      :limit="PAGE_LIMIT"
      :see-all="false"
      @select="(group) => group.traceId && onSelect(group.traceId)"
    />

    <RouterView />
  </div>
</template>
```

Replace the `hosts` placeholder above with the real `useInstances()` usage that matches the composable's current signature (read `src/composables/useInstances.ts` and map its instances to `host` strings — `InstanceDetail`'s host and `FailureGroup.instance` are both host names, and the select is an equality filter). `onSelect` is:

```ts
function onSelect(traceId: string) {
  void router.push({
    name: 'failure-detail',
    params: { traceId },
    query: toRouteQuery(draft.value),
  })
}
```

and lives in the `<script setup>` block.

- [ ] **Step 5: Let `FailureList` render the whole page**

`FailureList.vue` currently hard-codes `groups.slice(0, 5)` (`:24`) and the English header "Top failures"/"last window". Give it the props the two consumers need and extract the row so the page can show every field:

```ts
const props = defineProps<{
  groups: FailureGroup[]
  markers?: Set<string>
  /** Rows to render; 0 means "all of them". */
  limit?: number
  /** Render the "查看全部" link to the failure list page. */
  seeAll?: boolean
}>()
```

and in the template: `v-for="group in visibleGroups"` with

```ts
const visibleGroups = computed(() =>
  props.limit && props.limit > 0 ? props.groups.slice(0, props.limit) : props.groups,
)
```

the header becomes `{{ title }}` / `{{ subtitle }}` driven by two optional props so the overview keeps "Top failures" and the page passes nothing (no title — the filter bar above it already says what is on screen), and `seeAll` renders

```html
<RouterLink
  data-testid="see-all"
  :to="{ name: 'failures-list' }"
  class="focus-ring rounded-md px-2 py-1 text-xs text-accent hover:underline"
  >查看全部</RouterLink
>
```

Move the row markup into a new `doggy-dns-ui/src/components/FailureRow.vue` (deviation 6) with props `{ group: FailureGroup; analyzed: boolean; dense?: boolean }` and an `emit('select', group)`; `FailureList.vue` keeps the panel, header and empty state and renders `<FailureRow v-for="group in visibleGroups" … @select="…">`. In dense mode (the overview card) the row shows response code, truncated query, count and the analyzed badge; in full mode (the page) it adds `instance`, `authority`, `firstSeen`/`lastSeen` formatted with `msToDatetime`, `error` and `durationMs`. Keep `data-testid="failure-row"` and `data-testid="analyzed-badge"` on the row so the existing overview spec and e2e keep working.

- [ ] **Step 6: Register the nested routes**

`doggy-dns-ui/src/router/index.ts:19-23` becomes:

```ts
{
  path: '/failures',
  name: 'failures-list',
  component: () => import('@/views/FailuresView.vue'),
  children: [
    {
      path: ':traceId',
      name: 'failure-detail',
      component: () => import('@/views/FailureView.vue'),
    },
  ],
},
```

`FailureView.vue` loses its own outer padding in the template (the parent now provides it) but keeps its whole script block from Task 11; the drawer renders inside `FailuresView`'s `<RouterView/>`, which is what keeps the list mounted behind it. `close()` there stops pushing `{ name: 'overview' }` and returns to the list with the bracket intact, dropping only the analysis deep-link param:

```ts
function close() {
  const { analysis: _dropped, ...rest } = route.query
  void router.push({ name: 'failures-list', query: rest })
}
```

Rename the route in the Task 11 spec helper from `failure-detail` at path `/failures/:traceId` to the nested form, or keep the helper's own flat route table — it mounts the component directly, so only the pushed route name has to match.

Add the nav link in `doggy-dns-ui/src/components/SideRail.vue`, beside the existing `/`, `/instances`, `/sessions` entries and with the same markup the neighbours use:

```ts
{ to: '/failures', label: '失败列表', icon: TriangleAlert }
```

`import { TriangleAlert } from '@lucide/vue'` — match the import line the other four icons already use.

- [ ] **Step 7: Run the checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run FailuresView FailureList index 2>&1 | tail -40`
Expected: PASS.

Run: `cd doggy-dns-ui && bun run type-check && bun run lint && bun run build 2>&1 | tail -25`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add doggy-dns-ui/src/components/FailureFilterBar.vue doggy-dns-ui/src/views/FailuresView.vue doggy-dns-ui/src/router doggy-dns-ui/src/components/SideRail.vue doggy-dns-ui/src/views/FailureView.vue
git commit -m "$(cat <<'EOF'
feat: add the failure list page that owns the drawer URL

/failures keeps the full field set, a filter bar with absolute brackets up to 7
days, a host select, the uncovered-host notice and the 200-group cap warning.
The drawer is now its child route, so the URL is the only way to open it.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 13: Delete the second way to open the drawer

D5 is only true once `ui.drawerTraceId` is gone: a store-held trace id means two mounts, and two mounts means the copy-pasted deep link and the browser back button disagree with what is on screen.

**Files:**
- Modify: `doggy-dns-ui/src/stores/ui.ts:22,28-34` (delete `drawerTraceId`, `openFailure`, `closeFailure`)
- Modify: `doggy-dns-ui/src/views/OverviewView.vue:89-93,118-123`
- Delete: `doggy-dns-ui/src/composables/useFailures.ts`, `doggy-dns-ui/src/composables/usePolling.ts`
- Test: `doggy-dns-ui/src/stores/__tests__/ui.spec.ts` (if it exists), `doggy-dns-ui/src/views/__tests__/OverviewView.spec.ts`, `doggy-dns-ui/e2e/console.spec.ts`

**Interfaces:**
- Consumes: `failures-list` / `failure-detail` routes and `toRouteQuery` (Task 12).
- Produces: no `HandoffContext` change; the store keeps only `handoff`, `clearHandoff`, `rememberHandoff`, `openAiPanel`, `setAiPanelMode`, `aiPanelMode`.

- [ ] **Step 1: Write the failing store and overview tests**

In `OverviewView.spec.ts` (which mounts with `createPinia()` and `stubs: { TrendChart: true }` and no router), add the router and replace the drawer expectations:

```ts
it('routes a row click to the failure list with the window in the URL', async () => {
  const router = makeRouter()
  const wrapper = await mountOverview(router)

  await wrapper.find('[data-testid="failure-row"]').trigger('click')
  await flushPromises()

  expect(router.currentRoute.value.name).toBe('failure-detail')
  expect(router.currentRoute.value.params.traceId).toBeTruthy()
  expect(router.currentRoute.value.query.windowMinutes).toBe('60')
})

it('links the card header to the whole list', async () => {
  const wrapper = await mountOverview(makeRouter())
  await flushPromises()

  expect(wrapper.find('[data-testid="see-all"]').exists()).toBe(true)
})

it('never mounts a drawer from the store', async () => {
  const wrapper = await mountOverview(makeRouter())

  expect(wrapper.findComponent(FailureDrawer).exists()).toBe(false)
})
```

`makeRouter()` is a `createMemoryHistory` router with the same three routes the app uses (overview plus the nested `/failures` pair); the drawer mount assertion is the regression pin, so import the real `FailureDrawer` as the component being searched for rather than a stub of it.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd doggy-dns-ui && bun run test:unit -- --run OverviewView ui 2>&1 | tail -30`
Expected: FAIL — the overview still mounts `<FailureDrawer v-if="ui.drawerTraceId">` and clicking a row calls `openFailure`.

- [ ] **Step 3: Cut the store field**

Delete from `doggy-dns-ui/src/stores/ui.ts`: the `drawerTraceId` ref (`:22`), `openFailure` and `closeFailure` (`:28-34`), and their entries in the store's return object. Nothing else reads them once the overview stops calling them — verify with a grep for `drawerTraceId|openFailure|closeFailure` across `doggy-dns-ui/src` and fix any straggler rather than leaving a dangling reference.

- [ ] **Step 4: Point the overview at the router**

`OverviewView.vue:89-93` becomes:

```html
<FailureList
  :groups="groups"
  :markers="markers"
  :limit="5"
  see-all
  @select="(group) => group.traceId && onSelect(group.traceId)"
/>
```

with

```ts
function onSelect(traceId: string) {
  void router.push({ name: 'failure-detail', params: { traceId }, query: { windowMinutes: '60' } })
}
```

(the overview's own window is a fixed 60 minutes today, so the literal keeps the deep link honest; reuse `toRouteQuery(draft.value)` if the view later grows a window picker). Delete the `<FailureDrawer v-if="ui.drawerTraceId" …>` block (`:118-123`) and the `ui` accesses that only existed for it; `useUiStore()` stays if `handoff` or the panel state is still used by the view. Add `import { useRouter } from 'vue-router'`.

- [ ] **Step 5: Delete the dead composables**

`doggy-dns-ui/src/composables/useFailures.ts` (the unused `useFailureDetail`) and `doggy-dns-ui/src/composables/usePolling.ts` have no importers — confirm with a grep for `useFailures|usePolling` (the `useFailureList` name is a different symbol and stays), then `git rm` both files.

- [ ] **Step 6: Move the e2e proof onto the new page**

`doggy-dns-ui/e2e/console.spec.ts` currently opens the drawer from `/` via `failure-row` + Escape (24 lines, including the case named `failure drawer opens and closes`). Replace it with the three cases the round is actually about — they are URL-driven, so they stay independent of the machine's zone and clock:

```ts
test('filters the failure list by host over a week', async ({ page }) => {
  await page.goto('/failures?windowMinutes=10080&instance=dns-node-2')
  await page.waitForSelector('[data-testid="failure-row"]')

  const rows = page.locator('[data-testid="failure-row"]')
  expect(await rows.count()).toBeGreaterThan(0)
  for (const row of await rows.all()) {
    expect(await row.locator('[data-testid="row-instance"]').innerText()).toBe('dns-node-2')
  }
  // The unlabeled fixture row is unreachable, and the page says so.
  await expect(page.locator('[data-testid="uncovered-notice"]')).toContainText('没有主机标签')
})

test('a one-hour bracket hides the three-day-old group', async ({ page }) => {
  await page.goto('/failures?windowMinutes=60')
  await page.waitForSelector('[data-testid="failure-row"]')

  await expect(page.locator('[data-testid="failure-row"]')).not.toContainText('api.internal.')
})

test('the drawer opens only through the URL and keeps the bracket', async ({ page }) => {
  await page.goto('/failures?windowMinutes=10080')
  await page.click('[data-testid="failure-row"]')

  await expect(page).toHaveURL(/windowMinutes=10080/)
  await expect(page.locator('[data-testid="range-chip"]')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.locator('[data-testid="range-chip"]')).toHaveCount(0)
  await expect(page).toHaveURL(/\/failures\?windowMinutes=10080$/)
})
```

`FailureRow.vue` must therefore expose `data-testid="row-instance"` on its instance cell (full mode only). Add `[data-testid="handoff-button"]` / `[data-testid="investigate-button"]` to `FailureDrawer.vue` if they are not there yet — Task 10's specs already reach for them.

- [ ] **Step 7: Run the checks**

Run: `cd doggy-dns-ui && bun run test:unit -- --run 2>&1 | tail -30`
Expected: PASS.

Run: `cd doggy-dns-ui && bun run test:e2e 2>&1 | tail -40`
Expected: PASS. The Playwright config (`playwright.config.ts`) starts `CONSOLE_DEMO=1 cargo run --quiet` on 8787 and `bun run dev` on 5173, so the fixture spread from Task 7 is what these three cases actually assert against.

Run: `cd doggy-dns-ui && bun run type-check && bun run lint && bun run build 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add -A doggy-dns-ui/src doggy-dns-ui/e2e
git commit -m "$(cat <<'EOF'
feat: make the failure deep link the only way to open the drawer

drawerTraceId/openFailure/closeFailure are gone, the overview card links to
/failures, and the e2e cases now assert host filtering over a week plus the
bracket surviving a row click and Escape.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 14: Document the two ceilings and verify the page for real

The 10080-vs-1440 split is invisible unless it is written down (spec §6.1), and so is the fact that changing a bracket no longer invalidates a stored analysis (D14). The last task makes both claims readable, then proves the UI in a browser.

**Files:**
- Modify: `README.md:73-80` (Evidence API), `:108-114` (Analysis API), `:240-243` (deep link), plus the failure-list block

**Interfaces:**
- Consumes: everything above.
- Produces: documentation that Task 6's tool descriptions and Task 12's hint line must not contradict.

- [ ] **Step 1: Update the endpoint blocks**

In `README.md:76`, replace the evidence query line with:

```
GET /api/failures/{traceId}/evidence?from=&to=&windowMinutes=&domain=&responseCode=&instance=
```

and add under the paragraph at `:79-80`:

```markdown
`from`/`to` are epoch milliseconds and win over `windowMinutes` when both are
given; `windowMinutes` otherwise expands to `[to - n, to]` with `to` defaulting to
the server clock. The investigation chain (`/api/failures`, `/evidence`,
`/analysis`) accepts a bracket from 5 minutes to 10080 minutes (7 days) and answers
`400 invalid_request` — it never clamps — when the start is at or after the end, the
span is outside that band, or `to` is more than 60 seconds in the future. The
overview and instance views are the live class and still take a relative 5–1440
minute window.
```

`README.md:111` becomes:

```
POST /api/failures/{traceId}/analysis?from=&to=&windowMinutes=&domain=&responseCode=&instance=&force=
```

and the failure-list block gains the count endpoint:

```
GET /api/failures?from=&to=&windowMinutes=&domain=&responseCode=&instance=&limit=
GET /api/failures/uncovered?from=&to=&windowMinutes=&domain=&responseCode=&instance=
```

with one line saying `uncovered` returns `{"unlabeled": n}` — the in-scope failures with no `host_name`, which the instance equality on the list query cannot reach.

- [ ] **Step 2: Rewrite the deep-link and staleness paragraphs**

`README.md:240-243` becomes:

```markdown
The failure list lives at `/failures?from=&to=&windowMinutes=&domain=&responseCode=&instance=`,
and a row opens the drawer at `/failures/{traceId}?…&analysis=<id>`. The bracket is
the only drawer state: there is no store-held trace id, so reloading or sharing the
URL reproduces exactly what the operator saw. An unknown `analysis` id falls back to
a fresh analysis. The list shows a marker for traces that already have an
investigation and never triggers analysis itself.

The fingerprint covers the trace's evidence, not the bracket that was requested: logs
and spans match by `trace_id` and the metric window is centred on the trace itself.
Widening 1 hour to 7 days therefore reuses the stored analysis on purpose instead of
re-running the model on identical evidence. One consequence lands on every deployment
upgrading to this version: analyses recorded before the change carry a range in their
fingerprint, so they are reported stale the first time they are opened and re-run on
demand — nothing is rewritten in place.
```

- [ ] **Step 3: Run the full verification, backend then frontend**

Run: `cargo fmt --check && cargo clippy --all-targets -- --D warnings && cargo test 2>&1 | tail -40`
Expected: all three clean. If `cargo fmt --check` reports a diff, run `cargo fmt` and commit the formatting separately only if it touches files outside this round's.

Run: `cd doggy-dns-ui && bun run type-check && bun run lint && bun run test:unit -- --run && bun run build 2>&1 | tail -40`
Expected: all clean.

Run: `cd doggy-dns-ui && bun run test:e2e 2>&1 | tail -40`
Expected: PASS, including the three new cases from Task 13.

- [ ] **Step 4: Prove the page in a browser, in demo mode**

Run `CONSOLE_DEMO=1 cargo run --quiet` and `cd doggy-dns-ui && bun run dev` in the background, then drive the real page (browser tools) rather than trusting the specs:

1. `/failures?windowMinutes=10080&instance=dns-node-2` → every row is `dns-node-2`, the uncovered notice appears, and the hint line states the 7-day ceiling.
2. Click `7 天`, then move 起 back three days → the URL switches to `from`/`to`, polling stops (watch the Network tab for 30 seconds), and the drawer's range chip matches the inputs.
3. Click a row → the drawer opens with `?from=…&to=…` in the URL; Escape returns to the list with the bracket intact, and the browser back button lands on the same place.
4. In demo mode the drawer shows evidence but no verdict — demo mode deliberately has no analysis service (`src/server.rs:166-173`), so the panel must say 分析不可用 rather than hang. Confirm and do not treat it as a regression.
5. `/failures?from=10&to=20` → the invalid message is visible and no `/api/failures` request is issued.

Stop both servers afterwards and remove any scratch directory created for the check.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs: state the two span ceilings and the range-free fingerprint

The investigation chain takes 5 minutes to 7 days and rejects instead of
clamping; the overview and instance views keep 5-1440 minutes. Reusing a stored
analysis across overlapping brackets is documented as the intent, together with
the one-time stale that upgrading causes.

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

## Self-review notes

- **Spec coverage:** D1 (Tasks 1, 3, 6, 8), D2 (Tasks 1, 3, 6, 14), D3 (Task 1's constant), D4 (Tasks 1, 3, 8, 11), D5 (Tasks 12, 13), D6 (Tasks 4, 7, 9, 12), D7 (nothing builds series for the chain and Task 12's page deliberately omits `TrendChart`), D8 (Tasks 9, 13), D9 (Task 6, with the `get_overview` carve-out in deviation 2), D10 (Task 7), D11 (Task 5), D12 (no task touches `PROMPT_VERSION`), D13 (empty results, pinned by Task 7's `failure_detail_respects_the_bracket`), D14 (Tasks 1, 2 Step 1, 14). §6.1 → deviation 1; §6.2 → Tasks 4, 9, 12; §6.3 → untouched by design; §6.4 → Task 6; §6.5 → Tasks 8, 10, 12, 13. §7's list maps to `domain_test` (1), `api_test` (3), `failures_test` (4), `evidence_test` (5), `tools_test` (6), `fixture_test` (7), `failureFilter.spec`/`useFailureList.spec`/`FailureView.spec`/`FailuresView.spec`/`OverviewView.spec` (8-13), `e2e/console.spec.ts` (13), README (14).
- **Placeholder scan:** every code step carries real code. Two instructions deliberately say "match the existing file" — Task 7 Step 1's `service()` helper and Task 12 Step 4's `hosts` mapping — because both read code already in the tree; each names the file and the shape to produce.
- **Type consistency:** `Range { start_ms, end_ms }` → wire `startMs`/`endMs`; `InvestigationFilter { from, to, domain, responseCode, instance }` is used identically in Tasks 8, 10, 11, 12; `useFailureList(draft: Ref<FilterDraft>, limit)` and `useInvestigation(traceId, filter)` keep one arity at every call site; `unlabeled` is the single JSON field name in Tasks 4, 9, 14.