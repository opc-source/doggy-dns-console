# 看板与实例详情的 7 天区间 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `/api/overview` 与 `/api/instances/{id}` 接受与回溯链同一套 5 分钟–7 天区间(拒绝而不 clamp),并把时间控件统一到看板、失败列表、实例抽屉三处。

**Architecture:** 时间契约仍只有一份:HTTP 参数 → `Range::resolve`/`Range::checked`(400,不 clamp)→ 数据源。看板新增"分桶"维度:`Range` 上三个纯函数决定 range query 的 step 与 rate 窗口,趋势线按分桶采样、p95 取分桶最差、总量仍是覆盖整段的单点 `increase`。实例详情的失败数从不封顶的 `COUNT(*)` 来,拿不到就是 `null`。

**Tech Stack:** Rust edition 2024 / axum 0.8 / reqwest+wiremock / Vue 3 `<script setup>` + TS + vue-router + Tailwind v4 + vitest + Playwright(bun)。

**Spec:** `docs/superpowers/specs/2026-11-03-absolute-range-failure-list-design.md`(2026-11-04 修订:D2 改写、D15/D16/D17 新增)。本计划实现的是那次修订;D1–D14 的既有代码已经在 PR #11 的分支上。

## Global Constraints

- 只读 console:不写配置、不碰 DNS、不管告警。
- API key / 地址只从环境变量来;`.env` 永不入库、永不上命令行;提交信息里不出现密钥。
- `MIN_RANGE_MINUTES = 5`、`MAX_RANGE_MINUTES = 10080`、`FUTURE_TOLERANCE_MS = 60_000`、`DEFAULT_WINDOW_MINUTES = 60` 保持 `src/domain.rs` 里的同一份常量,前端 `failureFilter.ts` 继续镜像它们。
- 区间非法一律 `400 invalid_request`,**不 clamp、不静默修**;`windowMinutes` 是 `u32`(负数在参数层就 400),`from`/`to` 是 `i64`。
- 每个 AI 协助的 commit 结尾加 `Co-authored-by: Qoder <noreply@qoder.com>`。
- `PROMPT_VERSION` 保持 `failure-analyst/v2`;只改 copilot 业务工具的 description 文本。
- 默认 1 小时看板的**趋势查询文本与 step 必须逐字节不变**(D15);p95 口径必然变,要变在文档与 hint 里(D16)。
- 绝对区间停 15s 自动轮询,只留手动刷新(D8)。
- 测试命令:`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`;前端在 `doggy-dns-ui/` 下 `bun run type-check`、`bun run lint`、`bun run test:unit -- --run`、单个文件用 `bun x vitest run <path>`、e2e `bun run test:e2e`。
- 每个新断言先证明它会红(改动被守住的那一行),再变绿。

---

### Task 1: `Range` 的分桶函数

**Files:**
- Modify: `src/domain.rs`(`MAX_RANGE_MINUTES` 附近加常量,`impl Range` 加四个方法)
- Modify: `tests/domain_test.rs`(加分桶表驱动用例)

**Interfaces:**
- Consumes: 既有 `Range { start_ms, end_ms }`。
- Produces: `Range::span_minutes() -> u32`、`Range::bucket_minutes() -> u32`、`Range::rate_window_minutes() -> u32`、`Range::step_seconds() -> u64`、`Range::whole_step_seconds() -> u64`;常量 `MAX_SERIES_POINTS: u32 = 168`、`MIN_RATE_WINDOW_MINUTES: u32 = 5`、`MAX_BUCKET_MINUTES: u32 = 60`。Task 2 的查询构造与 Task 4 的 fixture 都靠它们算同一个数。

- [ ] **Step 1: 写失败的测试**

在 `tests/domain_test.rs` 追加(文件顶部已有 `use doggy_dns_console::domain::{...}`;把 `Range` 加进去):

```rust
#[test]
fn bucket_width_tiles_a_bracket_into_at_most_168_points() {
    let bracket = |minutes: i64| Range::new(0, minutes * 60_000);
    assert_eq!(bracket(5).bucket_minutes(), 1);
    assert_eq!(bracket(60).bucket_minutes(), 1);
    assert_eq!(bracket(168).bucket_minutes(), 1);
    assert_eq!(bracket(169).bucket_minutes(), 2);
    assert_eq!(bracket(1_440).bucket_minutes(), 9);
    assert_eq!(bracket(10_080).bucket_minutes(), 60);
    assert_eq!(bracket(10_080).step_seconds(), 3_600);
}

#[test]
fn rate_window_never_drops_below_five_minutes() {
    let bracket = |minutes: i64| Range::new(0, minutes * 60_000);
    assert_eq!(bracket(60).rate_window_minutes(), 5);
    assert_eq!(bracket(1_440).rate_window_minutes(), 9);
    assert_eq!(bracket(10_080).rate_window_minutes(), 60);
}

#[test]
fn totals_are_sampled_once_over_the_whole_bracket() {
    assert_eq!(Range::new(0, 60 * 60_000).whole_step_seconds(), 3_600);
    assert_eq!(Range::new(0, 10_080 * 60_000).whole_step_seconds(), 604_800);
    assert_eq!(Range::new(0, 5 * 60_000).span_minutes(), 5);
    assert_eq!(Range::new(0, 90_000).span_minutes(), 2);
}
```

最后一条里的 `span_minutes(90_000) == 2` 钉住"向上取整":一分钟半的区间不能算成 1 分钟。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test domain_test`
Expected: 编译失败,`no method named 'bucket_minutes' found for struct 'Range'`。

- [ ] **Step 3: 写实现**

在 `src/domain.rs` 的 `MAX_RANGE_MINUTES` 声明之后加:

```rust
/// The trend chart stays readable at about this many points; the bucket width
/// is derived from it, never tuned per query.
pub const MAX_SERIES_POINTS: u32 = 168;
/// The narrowest rate window a trend query uses.
pub const MIN_RATE_WINDOW_MINUTES: u32 = 5;
/// The widest bucket: seven days at one point per hour.
pub const MAX_BUCKET_MINUTES: u32 = 60;
```

在 `impl Range` 里 `checked` 之后加:

```rust
    /// Whole minutes covered by the bracket, rounded up so a sub-minute sliver
    /// still gets its own bucket instead of a zero-width one.
    pub fn span_minutes(&self) -> u32 {
        let span_ms = self.end_ms.saturating_sub(self.start_ms).max(0);
        ((span_ms + 59_999) / 60_000).max(1) as u32
    }

    /// Bucket width the trend and p95 queries share (spec D15). A one-hour
    /// bracket gets `1`, so the default chart keeps the 60-second step it has
    /// always used.
    pub fn bucket_minutes(&self) -> u32 {
        self.span_minutes()
            .div_ceil(MAX_SERIES_POINTS)
            .clamp(1, MAX_BUCKET_MINUTES)
    }

    /// Rate window for the trend and p95 queries: never narrower than five
    /// minutes, which is what keeps the default one-hour curve byte-identical
    /// to the pre-bracket version.
    pub fn rate_window_minutes(&self) -> u32 {
        self.bucket_minutes().max(MIN_RATE_WINDOW_MINUTES)
    }

    /// Range-query step, in seconds, that tiles the bracket into buckets.
    pub fn step_seconds(&self) -> u64 {
        u64::from(self.bucket_minutes()) * 60
    }

    /// Range-query step, in seconds, for a single sample covering the bracket.
    /// Totals use this so a seven-day chart does not replay the same
    /// `increase` window ten thousand times.
    pub fn whole_step_seconds(&self) -> u64 {
        u64::from(self.span_minutes()) * 60
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --test domain_test && cargo clippy --all-targets -- -D warnings`
Expected: PASS,无 clippy 警告。

- [ ] **Step 5: 红/绿自查**

把 `rate_window_minutes` 临时改成 `self.bucket_minutes()`(去掉 `.max`),确认 `rate_window_never_drops_below_five_minutes` 变红,然后改回。

- [ ] **Step 6: 提交**

```bash
git add src/domain.rs tests/domain_test.rs
git commit -m "$(cat <<'EOF'
feat: derive the trend bucket from the requested bracket

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 2: 看板查询按区间与分桶跑

**Files:**
- Modify: `src/datasource/mod.rs`(`TelemetrySource::overview` 签名 `Window` → `Range`)
- Modify: `src/datasource/openobserve/overview.rs`(`total_query`/`failure_query`/`p95_query`/`series_query`/`failure_series_query`)
- Modify: `src/datasource/openobserve/telemetry.rs`(`overview`、新增 `worst_value`、删掉只为 overview 存在的 `range(window)` 调用)
- Modify: `src/datasource/fixture.rs`(`overview` 按区间铺点)
- Modify: `tests/overview_test.rs`、`tests/fixture_test.rs`、`tests/api_evidence_test.rs:31`(`FailingTelemetry`)、`tests/instances_test.rs:115`(`SeenOnly`)

**Interfaces:**
- Consumes: Task 1 的 `Range::rate_window_minutes`/`step_seconds`/`whole_step_seconds`/`span_minutes`。
- Produces: `async fn overview(&self, range: Range) -> Result<Overview, DataError>`;`Overview.window_minutes` 从此是**区间跨度分钟数**;`series_query(metric: &str, rate_window_minutes: u32)`、`failure_series_query(metric: &str, rate_window_minutes: u32)`、`p95_query(metric: &str, rate_window_minutes: u32)`、`total_query(metric: &str, range: Range)`、`failure_query(metric: &str, range: Range)`。Task 3 依赖这个新签名。

- [ ] **Step 1: 写失败的测试(查询文本与 step)**

`tests/overview_test.rs` 的现成形状:`config(base_url)`(metric 名是 `dns_queries` / `dns_query_duration_bucket`)、`matrix(values, labels)`、`mount(server, query, body)`、`ts(base)`(ms→秒)、`telemetry(server)`(内部 clock = `1_761_436_800_000`)。三条新用例照它写,`Range` 从 `domain` 导入(该文件现在导入的是 `Window`)。

先补一个能钉 step 的挂载 helper(现有 `mount` 只按 `query` 匹配,而 D15 的另一半是 step):

```rust
/// `query_promql` is a GET with `query`/`start`/`end`/`step`; matching on
/// `step` too is the only way to pin D15's other half — the same query text
/// replayed every 60s over a week is still 10080 samples.
async fn mount_at_step(
    server: &MockServer,
    query: &str,
    step_seconds: u64,
    body: ResponseTemplate,
) {
    Mock::given(method("GET"))
        .and(path("/api/default/prometheus/api/v1/query_range"))
        .and(query_param("query", query))
        .and(query_param("step", step_seconds.to_string()))
        .respond_with(body)
        .mount(server)
        .await;
}
```

```rust
#[tokio::test]
async fn a_one_hour_bracket_keeps_the_existing_trend_text_and_step() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    // The default view must not change shape: `rate[5m]` at a 60s step, and
    // totals still one `increase(m[60m])` covering the bracket.
    let rate = series_query("dns_queries", 5);
    let failure_rate = failure_series_query("dns_queries", 5);
    let p95 = p95_query("dns_query_duration_bucket", 5);
    assert_eq!(rate, "sum(rate(dns_queries[5m])) * 60");
    assert_eq!(
        p95,
        "histogram_quantile(0.95, sum by (le) (rate(dns_query_duration_bucket[5m])))"
    );
    let window = Range::new(now - 3_600_000, now);
    mount_at_step(&server, &total_query("dns_queries", window), 3_600, matrix(serde_json::json!([[ts(now), "1000"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_query("dns_queries", window), 3_600, matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &p95, 60, matrix(serde_json::json!([[ts(now), "12.4"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &rate, 60, matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_rate, 60, matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({}))).await;

    let overview = telemetry(&server)
        .overview(window)
        .await
        .unwrap();
    assert_eq!(overview.window_minutes, 60);
    assert_eq!(overview.queries, 1000);
    assert_eq!(overview.p95_latency_ms, Some(12.4));
}

#[tokio::test]
async fn a_week_bracket_widens_the_rate_window_and_the_step() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    // 10080 / 168 = 60-minute buckets: hourly points, and the totals still
    // sampled once for the whole week instead of every minute.
    let window = Range::new(now - 10_080 * 60_000, now);
    mount_at_step(&server, &series_query("dns_queries", 60), 3_600, matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_series_query("dns_queries", 60), 3_600, matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &p95_query("dns_query_duration_bucket", 60), 3_600, matrix(serde_json::json!([[ts(now), "9"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &total_query("dns_queries", window), 604_800, matrix(serde_json::json!([[ts(now), "1000"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_query("dns_queries", window), 604_800, matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({}))).await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.window_minutes, 10_080);
    assert_eq!(total_query("dns_queries", window), "sum(increase(dns_queries[10080m]))");
}

#[tokio::test]
async fn p95_reports_the_worst_finite_bucket_not_the_last_one() {
    let server = MockServer::start().await;
    let now = 1_761_436_800_000_i64;
    let window = Range::new(now - 3_600_000, now);
    // Peaks in the middle and ends on NaN: `histogram_quantile` over an empty
    // bucket is NaN, so the old "last point" rule would have reported it.
    let points = serde_json::json!([
        [ts(now - 180_000), "12.5"],
        [ts(now - 120_000), "42.5"],
        [ts(now - 60_000), "7.5"],
        [ts(now), "NaN"]
    ]);
    mount_at_step(&server, &series_query("dns_queries", 5), 60, matrix(serde_json::json!([[ts(now), "20"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_series_query("dns_queries", 5), 60, matrix(serde_json::json!([[ts(now), "2"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &p95_query("dns_query_duration_bucket", 5), 60, matrix(points, serde_json::json!({}))).await;
    mount_at_step(&server, &total_query("dns_queries", window), 3_600, matrix(serde_json::json!([[ts(now), "1000"]]), serde_json::json!({}))).await;
    mount_at_step(&server, &failure_query("dns_queries", window), 3_600, matrix(serde_json::json!([[ts(now), "12"]]), serde_json::json!({}))).await;

    let overview = telemetry(&server).overview(window).await.unwrap();
    assert_eq!(overview.p95_latency_ms, Some(42.5));
}
```

`"NaN"` 在 JSON 里是字符串,`PromSeries` 的取值路径(`src/datasource/openobserve/client.rs` 的 matrix 解析)会把它解成 `f64::NAN` 还是解不出来直接跳过——**先读那段解析代码再定用例**:若解析层用 `as_f64`/`parse::<f64>()`,`"NaN"` 会解成 `f64::NAN`(`str::parse` 认 "NaN"),上面这条用例就直接达标;若解析层把无法取值的点丢掉,就把它改成"最后一点是一个明显更小的有限值(7.5)配中间峰值 42.5",并在 `worst_value` 上加一个 `#[cfg(test)]` 单测喂 `&[PromSeries]` 含 `f64::NAN` 的点。两种都算钉住 D16,但不许两种都跳过。


- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test overview_test`
Expected: 编译失败(`overview` 仍收 `Window`),或既有断言失败。

- [ ] **Step 3: 改查询构造**

`src/datasource/openobserve/overview.rs`:

```rust
use crate::domain::Range;

pub fn total_query(metric: &str, range: Range) -> String {
    scoped_total_query(metric, range.span_minutes(), None)
}

pub fn failure_query(metric: &str, range: Range) -> String {
    scoped_failure_query(metric, range.span_minutes(), None)
}

/// p95 over ONE bucket (spec D16): the caller samples it at `step_seconds`,
/// so the series is one number per bucket and the report takes the worst.
pub fn p95_query(metric: &str, rate_window_minutes: u32) -> String {
    format!(
        "histogram_quantile(0.95, sum by (le) (rate({metric}[{rate_window_minutes}m])))"
    )
}

pub fn series_query(metric: &str, rate_window_minutes: u32) -> String {
    format!("sum(rate({metric}[{rate_window_minutes}m])) * 60")
}

pub fn failure_series_query(metric: &str, rate_window_minutes: u32) -> String {
    format!(
        "sum(rate({metric}{{dns_response_code=~\"SERVFAIL|REFUSED|FORMERR\"}}[{rate_window_minutes}m])) * 60"
    )
}
```

删掉 `series_query(metric, failures: bool)` 的 bool 分支(它现在只有一个调用方,`failure_series_query` 直接用)。`scoped_total_query`/`scoped_failure_query`/`promql_label_value`/`seen_query` 不动——`scoped_*` 仍被 `failure_metrics` 用。

- [ ] **Step 4: 改 overview 实现**

`src/datasource/openobserve/telemetry.rs`:

```rust
    async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        let (start, end) = (range.start_ms, range.end_ms);
        let cfg = self.client.config();
        let rate_window = range.rate_window_minutes();
        let step = range.step_seconds();
        // Totals describe the whole bracket, so they ask for one sample at the
        // end. A 60-second step over seven days would replay the same
        // `increase` window 10080 times to read its last point (spec D15).
        let whole = range.whole_step_seconds();
        let total = total_query(&cfg.metric_queries, range);
        let failure = failure_query(&cfg.metric_queries, range);
        let p95 = p95_query(&cfg.metric_duration, rate_window);
        let volume = series_query(&cfg.metric_queries, rate_window);
        let failure_volume = failure_series_query(&cfg.metric_queries, rate_window);

        let queries = self.client.query_promql(&total, start, end, whole);
        let failures = self.client.query_promql(&failure, start, end, whole);
        let p95 = self.client.query_promql(&p95, start, end, step);
        let volume_series = self.client.query_promql(&volume, start, end, step);
        let failure_series = self.client.query_promql(&failure_volume, start, end, step);
        ...（`tokio::join!` 与错误传播保持原样）
        let p95_value = worst_value(&p95).map(|value| (value * 100.0).round() / 100.0);
        ...
        Ok(Overview {
            window_minutes: range.span_minutes(),
            ...
        })
    }
```

在 `last_value` 附近加:

```rust
/// The worst finite sample in a p95 series (spec D16): a bucket with no
/// traffic answers NaN, and reporting the last point would report that NaN.
fn worst_value(series: &[PromSeries]) -> Option<f64> {
    series
        .first()?
        .points
        .iter()
        .filter_map(|(_, value)| value.is_finite().then_some(*value))
        .max_by(f64::total_cmp)
}
```

`fn range(&self, window: Window)` 只有 `seen_instances`(:240)继续用时保留,并把它上面那条注释改成"发现查询仍按相对窗口(D2:`Window` 只剩内部用途)";若已无人使用则删除。

- [ ] **Step 5: 改 fixture**

`src/datasource/fixture.rs`:

```rust
    /// Demo trend: the bracket tiled by its own bucket width, so a 1-hour view
    /// still gets 60 points and a week gets ~168. The load formula is the one
    /// this fixture has always used; only the sampling grid follows the range.
    async fn overview(&self, range: Range) -> Result<Overview, DataError> {
        let bucket_ms = i64::from(range.bucket_minutes()) * 60_000;
        let mut series = Vec::new();
        let mut ts = range.end_ms;
        while ts > range.start_ms {
            series.push(SeriesPoint {
                ts,
                queries: 4_800 + (series.len() as u64 * 137 % 1_600),
                failures: 8 + (series.len() as u64 * 7 % 31),
            });
            ts = ts.saturating_sub(bucket_ms);
        }
        series.reverse();
        let queries = series.iter().map(|point| point.queries).sum();
        let failures = series.iter().map(|point| point.failures).sum();
        Ok(Overview {
            window_minutes: range.span_minutes(),
            instances_online: 0,
            instances_total: 0,
            queries,
            failures,
            success_rate: Overview::success_rate(queries, failures),
            p95_latency_ms: Some(12.4),
            series,
        })
    }
```

- [ ] **Step 6: 更新既有测试与 stub**

- `tests/overview_test.rs:64` 那条 `overview_computes_success_rate_and_series_from_promql`:`Window::new(60)` 换成 `Range::new(now - 3_600_000, now)`,构造调用全部跟到新签名(`total_query(m, range)`、`series_query(m, 5)`、`failure_series_query(m, 5)`、`p95_query(m, 5)`),`window_minutes` 断言仍是 60。`grep -n "series_query\|p95_query\|total_query\|failure_query\|Window" tests/*.rs src/*.rs src/**/*.rs` 要把**所有**调用点清干净(`failure_metrics` 用 `scoped_*`,签名不变)。
- `tests/fixture_test.rs:28` `fixture_overview_has_nodes_and_success_rate`:改成"totals 由 series 求和"这条新钉:`assert_eq!(overview.queries, overview.series.iter().map(|p| p.queries).sum::<u64>())`,并加一条"7 天区间点数 > 1 小时点数"。
- `tests/fixture_test.rs:63` `fixture_instance_detail_counts_recent_failures` 先改成 `detail.failure_count`(**Task 4 才引入该字段**;此任务里若字段还不存在,只改 overview 相关断言,把这条留给 Task 4)——不要为了跑绿而提前改字段名。
- `tests/api_evidence_test.rs:31`、`tests/instances_test.rs:115` 两个 stub:`async fn overview(&self, _range: Range) -> Result<Overview, DataError>`。
- `src/service.rs:38` `overview(window: Window)` → `overview(range: Range)`(本任务先把它直通给数据源,Task 3 再让 HTTP 传区间;此步骤只改签名保证编译)。

Run: `cargo test --test overview_test --test fixture_test --test domain_test`
Expected: PASS。

- [ ] **Step 7: 全量 Rust gate + 提交**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: 全绿(既有 274 条不回归)。

```bash
git add src/datasource src/service.rs tests/overview_test.rs tests/fixture_test.rs tests/api_evidence_test.rs tests/instances_test.rs
git commit -m "$(cat <<'EOF'
feat: sample the overview by bucket and report the worst p95 bucket

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 3: `/api/overview` 与 `get_overview` 收区间

**Files:**
- Create/Modify: `src/api/mod.rs`(新增共用 `RangeQuery`)
- Modify: `src/api/overview.rs`、`src/agent/tools.rs`(`OverviewArgs` + `get_overview` + 工具 description)
- Modify: `tests/api_test.rs`、`tests/tools_test.rs`(`:29` 的 `OverviewArgs { window_minutes: Some(60) }` 是**完备字面量**,加字段后必须补 `..OverviewArgs::default()` 才编译;`:100-105` 那条 `!overview.description().contains("10080")` 是 D2 改写前的旧口径,见 Step 5)

**Interfaces:**
- Consumes: Task 2 的 `overview(Range)`;既有 `Range::resolve`、`DataError::Invalid`、`ApiError` 映射。
- Produces: `crate::api::RangeQuery { from: Option<i64>, to: Option<i64>, window_minutes: Option<u32> }` + `RangeQuery::range(&self, now_ms: i64) -> Result<Range, DataError>`,Task 4 的 `/api/instances/{id}` 复用它。

- [ ] **Step 1: 写失败的测试(HTTP)**

`tests/api_test.rs` **没有** `get(url)` 这种 helper:它的形状是 `app_with(...) -> Router` + `Request::get(uri).body(Body::empty())` + `tower::ServiceExt::oneshot` + `json_body(response)`,时钟固定 `FixedClock(1_761_436_800_000)`(`tests/api_test.rs:85-122`)。照那个形状写,并把时钟写成同一个常量:

```rust
const FIXED_NOW_MS: i64 = 1_761_436_800_000;

#[tokio::test]
async fn overview_takes_an_absolute_bracket_without_clamping_it() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get(&format!(
                "/api/overview?from={}&to={}",
                FIXED_NOW_MS - 1_000 * 60_000,
                FIXED_NOW_MS
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    // 1000 分钟在 10080 上限之内,却远在旧的 5-1440 clamp 之外:
    // 桥接实现(Task 2 留的)会在这里答 60,断言 `windowMinutes` 才抓得住它。
    assert_eq!(body["windowMinutes"], 1_000);
}

#[tokio::test]
async fn overview_takes_a_week_from_the_relative_shortcut() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/overview?windowMinutes=10080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["windowMinutes"], 10_080);
}

#[tokio::test]
async fn overview_refuses_a_bracket_the_investigation_chain_refuses() {
    // 四条规则、同一个 400,而且最重要的是:不 clamp。
    // `windowMinutes=20000` 过去会悄悄变成 1440。
    for uri in [
        "/api/overview?windowMinutes=20000",
        "/api/overview?from=1000&to=2000",
        "/api/overview?from=2000&to=1000",
        &format!("/api/overview?to={}", FIXED_NOW_MS + 61_000),
    ] {
        let app = app_with(fixture() as Arc<dyn TelemetrySource>);
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = json_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request", "{uri}: {body}");
    }
}

#[tokio::test]
async fn a_negative_window_minutes_is_refused_at_the_parameter_layer() {
    let app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let response = app
        .oneshot(
            Request::get("/api/overview?windowMinutes=-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // `window_minutes` 是 `u32`,负数在 axum 的反序列化层就 400,压根进不到
    // `Range::resolve`;因此只钉状态码,不钉 `{error:{code,message}}` 信封。
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

最后一条**先跑一次,把真实响应体贴进报告**:如果你看到它其实走的是 `{error:{code:"invalid_request"}}` 信封(axum 装了自定义 rejection 的话),就把断言升级为信封断言并在报告里说明——不要凭猜写注释。

- [ ] **Step 2: 写失败的测试(两端同文案)**

```rust
#[tokio::test]
async fn the_overview_and_the_failure_list_refuse_identically() {
    let overview_app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let overview = json_body(
        overview_app
            .oneshot(
                Request::get("/api/overview?windowMinutes=20000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let failures_app = app_with(fixture() as Arc<dyn TelemetrySource>);
    let failures = json_body(
        failures_app
            .oneshot(
                Request::get("/api/failures?windowMinutes=20000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        overview["error"]["message"],
        "invalid request: range is longer than the 10080-minute ceiling"
    );
    assert_eq!(overview, failures);
}
```

(`app_with` 返回的 `Router` 被 `oneshot` 按值吃掉,所以两端各建一个 app。)

- [ ] **Step 3: 实现 `RangeQuery`**

`src/api/mod.rs`:

```rust
use crate::domain::Range;
use crate::error::DataError;
use serde::Deserialize;

/// The bracket grammar every time-ranged endpoint reads: `from`/`to` in epoch
/// milliseconds win over `windowMinutes`, a lone bound still participates, and
/// an illegal span is a `400` — never a clamp (spec D2). `FailureQuery` and
/// `StartQuery` keep their own copies of these three fields because they also
/// carry domain/responseCode/instance; reusing them here would advertise
/// filters the overview does not have.
#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    #[serde(rename = "windowMinutes")]
    pub window_minutes: Option<u32>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl RangeQuery {
    pub fn range(&self, now_ms: i64) -> Result<Range, DataError> {
        Range::resolve(self.from, self.to, self.window_minutes, now_ms)
            .map_err(DataError::Invalid)
    }
}
```

`src/api/overview.rs` 整体换成下面这份(`WindowQuery` 是它的私有类型,删掉前先
`grep -rn "WindowQuery" src tests doggy-dns-ui/src` 确认只有这一处引用;`Window`/
`Range::from_window` 的 import 随之消失,但 `Window` 本身还有内部用途,别动 `src/domain.rs`):

```rust
use axum::Json;
use axum::extract::{Query, State};

use crate::api::error::ApiError;
use crate::api::state::ApiState;
use crate::api::RangeQuery;
use crate::domain::Overview;

pub async fn overview(
    State(state): State<ApiState>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<Overview>, ApiError> {
    let range = query.range(state.service.now_ms())?;
    Ok(Json(state.service.overview(range).await?))
}
```

- [ ] **Step 4: 实现 copilot 工具**

`src/agent/tools.rs`:`OverviewArgs`(`:13-17`)加两个字段并改 `get_overview`(`:61-74`)。**保持现有的 derive 与 `#[serde(rename_all = "camelCase")]` 不动**——`build_tools` 用 `.with_parameters_schema::<OverviewArgs>()`,少了 `JsonSchema` 直接编译不过;`Default` 被 `..OverviewArgs::default()` 的测试用法依赖。字段顺序与 `FailureArgs` 一致:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverviewArgs {
    /// Relative shortcut; expanded to `[to - n, to]`. Used to derive the start
    /// when from is absent; ignored when from is given.
    pub window_minutes: Option<u32>,
    /// Absolute bracket start in epoch milliseconds. Wins over `windowMinutes`
    /// when `to` is present too.
    pub from: Option<i64>,
    /// Absolute bracket end in epoch milliseconds; also anchors `windowMinutes`
    /// when `from` is absent. Defaults to the server clock.
    pub to: Option<i64>,
}

pub async fn get_overview(
    service: &ConsoleService,
    args: OverviewArgs,
) -> Result<Value, DataError> {
    let range = Range::resolve(args.from, args.to, args.window_minutes, service.now_ms())
        .map_err(DataError::Invalid)?;
    let overview = service.overview(range).await?;
    serde_json::to_value(overview).map_err(|error| DataError::Invalid(error.to_string()))
}
```

最后一行的 `DataError::Invalid` 是**这个文件里 5 个工具共同的写法**(`get_failure_detail` 在 `:120` 同样用 `Invalid`):本期只动区间契约,不改串行化失败的状态码归类。别把它"顺手"换成 `Internal`。

工具 description 现在写着"only accepts the relative `windowMinutes` … clamps it to 5-1440 minutes"——Task 3 之后这句话就是假的(§6.4:描述里承诺的能力必须是代码真提供的能力)。整段换成:

```
Get fleet-wide DNS health for a time bracket: instances online/total, query volume, failure count, success rate, p95 latency and the trend series. Use this before quoting any KPI. Frame the window with from/to in epoch milliseconds (a span of 5 minutes up to 7 days, i.e. 10080 minutes) or with the windowMinutes shortcut; a bracket shorter than 5 minutes, longer than 10080 minutes, with a start at or after its end, or ending more than 60 seconds in the future is rejected, never clamped. P95 is the worst bucket inside the bracket, so a wider bracket reads higher.
```

- [ ] **Step 5: 钉住工具描述与参数**

先在 `tests/tools_test.rs` **删掉一条已经过期的钉子**:`:100-105` 现在断言 `!overview.description().contains("10080")`,注释写着"The live class keeps its 1440-minute ceiling and must not promise a week"。那是 D2 改写之前的口径,Task 3 之后看板确实能取一周——把它换成反向的钉子(见下面第一条),而不是留着它逼自己把描述里的 `10080` 抹掉。

再追加(照 `failure_tool_descriptions_state_the_absolute_range_and_the_real_ceilings` 的写法,`tools::build_tools(Arc::new(service.clone()))`、`tool.name()`、`tool.description()`;`fixture_service()` 返回 `ConsoleService` 本体,不是 `Arc`):

```rust
#[tokio::test]
async fn the_overview_tool_promises_the_bracket_it_actually_takes() {
    let service = fixture_service();
    let tools = tools::build_tools(Arc::new(service.clone()));
    let overview = tools
        .iter()
        .find(|tool| tool.name() == "get_overview")
        .expect("get_overview is registered");
    let text = overview.description();
    for needle in [
        "from",
        "to",
        "windowMinutes",
        "epoch milliseconds",
        "7 days",
        "10080",
        "worst bucket",
        "rejected",
    ] {
        assert!(text.contains(needle), "get_overview description lost `{needle}`: {text}");
    }
    // 反向钉子:描述不许再承诺 clamp,也不许再提 1440(那是看板不再有
    // 的旧 live 类上限)。
    assert!(!text.contains("1440"), "{text}");
    assert!(!text.contains("clamps"), "{text}");
}

#[tokio::test]
async fn get_overview_tool_takes_a_week_without_clamping_it() {
    let service = fixture_service();
    let value = tools::get_overview(
        &service,
        OverviewArgs {
            window_minutes: Some(10_080),
            ..OverviewArgs::default()
        },
    )
    .await
    .expect("a week is inside the ceiling");
    // 桥接实现会在这里报 1440。
    assert_eq!(value["windowMinutes"], 10_080);
}

#[tokio::test]
async fn get_overview_tool_refuses_a_week_plus_a_day() {
    let service = fixture_service();
    let error = tools::get_overview(
        &service,
        OverviewArgs {
            from: Some(1_761_436_800_000 - 11_000 * 60_000),
            to: Some(1_761_436_800_000),
            ..OverviewArgs::default()
        },
    )
    .await
    .expect_err("a bracket over the ceiling must not be clamped");
    assert!(error.to_string().contains("10080-minute ceiling"), "{error}");
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test --test api_test --test tools_test && cargo clippy --all-targets -- -D warnings`
Expected: PASS。

- [ ] **Step 7: 红/绿自查**

把 `RangeQuery::range` 临时换成 `Ok(Range::from_window(Window::new(query.window_minutes.unwrap_or(60)), now_ms))`(也就是把 Task 2 留下的桥接塞回来),确认这三条**全部**变红:
`overview_takes_an_absolute_bracket_without_clamping_it`(答 60 而不是 1000)、
`overview_takes_a_week_from_the_relative_shortcut`(clamp 成 1440 而不是 10080)、
`overview_refuses_a_bracket_the_investigation_chain_refuses`(`windowMinutes=20000` 变 200)。
再把 `get_overview` 的 `Range::resolve` 换回 `Range::from_window(Window::new(...))`,确认
`get_overview_tool_takes_a_week_without_clamping_it`(1440 ≠ 10080)与
`get_overview_tool_refuses_a_week_plus_a_day` 变红。然后全部改回。
只断言 `StatusCode::OK` 的第一条在桥接下也是 200,所以断言 `windowMinutes` 才是它的红点——别把它改回只查状态码。

- [ ] **Step 8: 全量 Rust gate + 提交**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

```bash
git add src/api src/agent/tools.rs tests/api_test.rs tests/tools_test.rs
git commit -m "$(cat <<'EOF'
feat: let the overview take the same bracket as the investigation chain

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 4: 实例详情按区间取一个不封顶的失败数

**Files:**
- Modify: `src/domain.rs`(`InstanceDetail.recent_failures` → `failure_count: Option<u64>`)
- Modify: `src/datasource/openobserve/failures.rs`(新 `instance_failure_count_query`)
- Modify: `src/datasource/mod.rs`(`TelemetrySource::instance_failure_count` 默认实现)
- Modify: `src/datasource/openobserve/telemetry.rs`、`src/datasource/fixture.rs`
- Modify: `src/datasource/mcp.rs:307`、`src/datasource/instances.rs:397`、`src/datasource/fixture.rs:391`(构造点)
- Modify: `src/service.rs`(`instance_detail(id, range)`)
- Modify: `src/api/instances.rs`、`src/agent/tools.rs`(`InstanceArgs` + `get_instance_detail` + 描述)
- Modify: `tests/fixture_test.rs`、`tests/instances_test.rs`、`tests/tools_test.rs`、`tests/api_test.rs`、`tests/evidence_test.rs:461`

**Interfaces:**
- Consumes: Task 3 的 `crate::api::RangeQuery`。
- Produces: `async fn instance_failure_count(&self, filter: FailureFilter) -> Result<u64, DataError>`;`InstanceDetail.failure_count: Option<u64>`(JSON `failureCount`,计不到数为 `null`);`ConsoleService::instance_detail(&self, id: &str, range: Range)`。

- [ ] **Step 1: 写失败的测试(SQL 文本)**

在 `tests/failures_test.rs` 追加(该文件已有直接调查询构造函数的先例):

```rust
#[test]
fn the_instance_count_is_an_uncapped_count_over_the_same_predicate() {
    let filter = FailureFilter {
        instance: Some("dns-node-1".to_owned()),
        ..FailureFilter::new(Range::new(1_000, 61_000))
    };
    let sql = instance_failure_count_query("doggy-dns-logs", &filter, 1_000, 61_000);
    assert!(sql.starts_with("SELECT COUNT(*) AS count"), "{sql}");
    // Same failure predicate as the list, the host equality folded in, and no
    // GROUP BY / LIMIT — a week must not truncate at 200 groups.
    assert!(sql.contains("severity IN ('WARN','ERROR')"), "{sql}");
    assert!(sql.contains("lower(host_name) = lower('dns-node-1')"), "{sql}");
    assert!(!sql.contains("GROUP BY"), "{sql}");
    assert!(!sql.contains("LIMIT"), "{sql}");
    assert!(sql.contains("_timestamp >= 1000000"), "{sql}");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --test failures_test`
Expected: 编译失败,`cannot find function instance_failure_count_query`。

- [ ] **Step 3: 实现 SQL 与数据源方法**

`src/datasource/openobserve/failures.rs`(放在 `unlabeled_failures_query` 之后,共用它的注释口径):

```rust
/// How many failure rows one host produced inside the bracket. The instance
/// detail reports this instead of summing list groups: a seven-day bracket
/// easily exceeds the 200-group cap, and a silently capped total would read as
/// "this node was healthy that week".
///
/// `filter.instance` is what scopes it — without one this is the whole bracket
/// across every host, which is the list's number, not a node's.
pub fn instance_failure_count_query(
    stream: &str,
    filter: &FailureFilter,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!(
        "SELECT COUNT(*) AS count FROM \"{stream}\" WHERE {FAILURE_PREDICATE}{} \
         AND _timestamp >= {} AND _timestamp <= {}",
        extra_filters(filter),
        start_ms.saturating_mul(1000),
        end_ms.saturating_mul(1000),
    )
}
```

`src/datasource/mod.rs` 在 `unlabeled_failures` 之后加:

```rust
    /// Failure rows for `filter.instance` (plus its domain/responseCode scope)
    /// inside the bracket, without the list's group cap. Sources that cannot
    /// count return an error rather than zero, which would read as "healthy".
    async fn instance_failure_count(&self, _filter: FailureFilter) -> Result<u64, DataError> {
        Err(DataError::NotConfigured(
            "instance_failure_count not implemented".to_string(),
        ))
    }
```

`src/datasource/openobserve/telemetry.rs` 实现它,照 `unlabeled_failures`(:150)的形状——同一条 `search_logs`、同一句"查不到整数就 warn 并记 0"的兜底,以及同样的 `max(0)`。区别只在 SQL 与 `1` 行:

```rust
    async fn instance_failure_count(&self, filter: FailureFilter) -> Result<u64, DataError> {
        let (start, end) = (filter.range.start_ms, filter.range.end_ms);
        let sql = instance_failure_count_query(&self.client.config().log_stream, &filter, start, end);
        let rows = self.client.search_logs(&sql, start, end, 1).await?;
        // (与 unlabeled_failures 同款取值 + warn)
    }
```

`src/datasource/fixture.rs` 实现:`self.groups()` 里 `self.matches(group, &filter)` 为真的 `count` 之和(它本来就按区间/主机/host 过滤,`matches` 已含 host 等值)。

- [ ] **Step 4: 改 service 与字段**

`src/domain.rs`:

```rust
    /// Failure rows attributed to this host inside the bracket the caller
    /// asked for. `None` when the telemetry source cannot count them — the
    /// drawer shows an em dash rather than a fabricated zero.
    pub failure_count: Option<u64>,
```

`src/service.rs`:

```rust
    pub async fn instance_detail(
        &self,
        id: &str,
        range: Range,
    ) -> Result<InstanceDetail, DataError> {
        let mut detail = self.instances.detail(id).await?;
        let filter = FailureFilter {
            instance: host_filter(Some(detail.instance.host.clone())),
            ..FailureFilter::new(range)
        };
        // A count the telemetry cannot produce stays `None`: the drawer renders
        // an em dash, because `0` would read as "this node was healthy".
        match self.telemetry.instance_failure_count(filter).await {
            Ok(count) => detail.failure_count = Some(count),
            Err(error) => tracing::warn!(
                instance = %detail.instance.id,
                error = %error,
                "instance failure count unavailable; reporting no number instead of 0"
            ),
        }
        Ok(detail)
    }
```

`Err` 分支**不许**写 `detail.failure_count = Some(0)`;`instances.detail(id)` 返回的构造点一律已经是 `None`,不动它就是在报"没有数"。构造点(`mcp.rs:307`、`instances.rs:397`、`fixture.rs:391`)把 `recent_failures: 0` 换成 `failure_count: None`。

`src/api/instances.rs`:

```rust
pub async fn instance_detail(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<InstanceDetail>, ApiError> {
    let range = query.range(state.service.now_ms())?;
    Ok(Json(state.service.instance_detail(&id, range).await?))
}
```

`src/agent/tools.rs`:`InstanceArgs` 加 `window_minutes`/`from`/`to`(与 `OverviewArgs` 同款三字段),`get_instance_detail` 用 `Range::resolve` 解析后传给 service,并把该工具的 description 补一句 `Pass the same from/to or windowMinutes the operator is looking at.`

- [ ] **Step 5: 更新既有测试**

- `tests/fixture_test.rs:62` `fixture_instance_detail_counts_recent_failures` 改名 `fixture_instance_detail_counts_failures_in_the_bracket`,断言:1 小时区间(`bracket(60)` 现成 helper)下 `failure_count == Some(n)` 且 7 天区间(`bracket(10_080)`)下 `7d > 1h` —— 这条钉子证明 fixture 真的按区间取数,而不是抄一个常量。
- `tests/evidence_test.rs:461` `instance_detail()` 构造体加 `failure_count: None`。
- `tests/instances_test.rs`、`tests/tools_test.rs:42`、`tests/api_test.rs` 里所有 `instance_detail(` 调用补区间参数。
- `tests/instances_test.rs` 加一条:数据源返回 `Err` 时 `failure_count` 是 `None`(用 `SeenOnly` 那个 stub 的同族写法),钉住"不发明 0"。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test --test failures_test --test fixture_test --test instances_test --test tools_test --test api_test --test evidence_test`
Expected: PASS。

- [ ] **Step 7: 全量 Rust gate + 提交**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

```bash
git add src tests
git commit -m "$(cat <<'EOF'
feat: count an instance's failures over the requested bracket

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 5: 把时间控件从筛选条抽出来共用

**Files:**
- Create: `doggy-dns-ui/src/components/TimeRangeControl.vue`
- Modify: `doggy-dns-ui/src/components/FailureFilterBar.vue`
- Create: `doggy-dns-ui/src/components/__tests__/TimeRangeControl.spec.ts`
- Modify: `doggy-dns-ui/src/components/__tests__/FailureFilterBar.spec.ts`、`doggy-dns-ui/src/views/FailuresView.vue`(若它直接依赖内部 testid)

**Interfaces:**
- Consumes: `@/lib/failureFilter` 的 `FilterDraft`/`relativeDraft`/`datetimeToMs`/`msToDatetime`/`DEFAULT_WINDOW_MINUTES`/`MIN_RANGE_MINUTES`/`MAX_RANGE_MINUTES`(**不改这个模块的行为**)。
- Produces: `<TimeRangeControl :model-value="draft" @update:model-value="..." />` —— 只负责时间那一段(预设 + 起/止 + 回到近 1 小时 + 口径提示),不碰 domain/responseCode/instance。Task 6、Task 7 各挂一个。

- [ ] **Step 1: 写失败的测试**

```ts
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import TimeRangeControl from '@/components/TimeRangeControl.vue'
import { emptyDraft } from '@/lib/failureFilter'

function bar(wrapper: ReturnType<typeof mount>) {
  return wrapper.find('[data-testid="range-hint"]').text()
}

describe('TimeRangeControl', () => {
  it('offers 1h, 6h, 24h and 7d presets and marks the active one', async () => {
    const wrapper = mount(TimeRangeControl, {
      props: { modelValue: { ...emptyDraft(), windowMinutes: 360 } },
    })
    for (const testId of ['quick-1h', 'quick-6h', 'quick-24h', 'quick-7d']) {
      expect(wrapper.find(`[data-testid="${testId}"]`).exists()).toBe(true)
    }
    expect(wrapper.get('[data-testid="quick-6h"]').attributes('aria-pressed')).toBe('true')
    await wrapper.get('[data-testid="quick-7d"]').trigger('click')
    const draft = wrapper.emitted('update:modelValue')?.at(-1)?.[0] as Record<string, unknown>
    expect(draft).toMatchObject({ fromMs: null, toMs: null, windowMinutes: 10080 })
  })

  it('never emits a half bracket', async () => {
    const wrapper = mount(TimeRangeControl, { props: { modelValue: emptyDraft() } })
    await wrapper.get('[data-testid="range-from"]').setValue('2026-01-01T00:00')
    const draft = wrapper.emitted('update:modelValue')?.at(-1)?.[0] as Record<string, unknown>
    expect(draft.fromMs).not.toBeNull()
    expect(draft.toMs).not.toBeNull()
  })

  it('states the one ceiling and the one endpoint without a window', () => {
    const wrapper = mount(TimeRangeControl, { props: { modelValue: emptyDraft() } })
    const hint = bar(wrapper)
    expect(hint).toContain('5')
    expect(hint).toContain('10080')
    // The old copy ("看板仍限 1440 分钟") is now false; the new copy must not
    // re-grow a per-endpoint number.
    expect(hint).not.toContain('1440')
    expect(hint).toContain('/api/instances')
  })
})
```

Run: `bun x vitest run src/components/__tests__/TimeRangeControl.spec.ts`
Expected: FAIL,组件不存在。

- [ ] **Step 2: 实现组件**

把 `FailureFilterBar.vue` 里时间那一段(script 里的 `fromText`/`toText`/`setFrom`/`setTo`/`useRelative`/`resetTime`/`activeQuick` 与模板里的预设组、两个 `datetime-local`、`回到近 1 小时`、`range-hint`)整体搬进 `TimeRangeControl.vue`,并给它加两个 `data-testid`:起 = `range-from`、止 = `range-to`。搬动时保持三处既有注释(成对区间 / 清空即回相对 / 预设必须丢掉绝对对)——那是行为钉子,不是装饰。

预设表(把看板的 6h 收进来,四处共用一份):

```ts
const QUICK_WINDOWS = [
  { label: '1 小时', minutes: 60, testId: 'quick-1h' },
  { label: '6 小时', minutes: 360, testId: 'quick-6h' },
  { label: '24 小时', minutes: 1440, testId: 'quick-24h' },
  { label: '7 天', minutes: MAX_RANGE_MINUTES, testId: 'quick-7d' },
]
```

提示文案(D2 改写后只有一句真话;数字全部插值,不手打):

```html
<p data-testid="range-hint" class="w-full text-[11px] text-muted">
  <!--
    One class now: every endpoint that takes a time range rejects outside
    {{ MIN_RANGE_MINUTES }}–{{ MAX_RANGE_MINUTES }} minutes instead of clamping
    (`Range::checked` in `src/domain.rs`), so the numbers below are the request
    contract, not a per-page ceiling. `/api/instances` is the one endpoint with
    no window at all — its rows are identity and reachability, so the hint has
    to name it or an operator will read a frozen instance list as a stale one.
  -->
  时间区间支持 {{ MIN_RANGE_MINUTES }} 分钟到 {{ MAX_RANGE_MINUTES }} 分钟（7 天），所有端点超限直接拒绝、不再夹回；
  实例列表（/api/instances）不取时间窗。
</p>
```

- [ ] **Step 3: 让筛选条只用它**

`FailureFilterBar.vue` 删掉搬走的片段与 `QUICK_WINDOWS`,改成 `<TimeRangeControl :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" />`,其余(实例/响应码/域名)不动。Run: `bun x vitest run src/components/__tests__/FailureFilterBar.spec.ts src/views/__tests__/FailuresView.spec.ts`
Expected: PASS(若断言写死了内部 DOM,更新到新的 testid,不改被测行为)。

- [ ] **Step 4: 红/绿自查 + gate + 提交**

把 `setFrom` 里"补全区间对"那一行临时删掉,确认 `never emits a half bracket` 变红;改回。
Run: `bun run type-check && bun run lint && bun run test:unit -- --run`

```bash
git add doggy-dns-ui/src/components
git commit -m "$(cat <<'EOF'
refactor: share one time-range control across the views

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 6: 看板按区间取数

**Files:**
- Modify: `doggy-dns-ui/src/composables/useOverview.ts`
- Modify: `doggy-dns-ui/src/views/OverviewView.vue`
- Create: `doggy-dns-ui/src/lib/chartLabels.ts`
- Modify: `doggy-dns-ui/src/components/TrendChart.vue`
- Modify: `doggy-dns-ui/src/views/__tests__/OverviewView.spec.ts`、`doggy-dns-ui/src/components/__tests__/KpiCard.spec.ts`(如需)

**Interfaces:**
- Consumes: Task 5 的 `TimeRangeControl`;`failureFilter.ts` 的 `parseFilterQuery`/`toRouteQuery`/`toRequestQuery`/`isAbsolute`/`validateDraft`/`validateBracket`/`resolveBracket`/`rangeLabel`;Task 3 的 `/api/overview?from&to`。
- Produces: `useOverview(draft: Ref<FilterDraft>)` → `{ data, error, invalid, loading, lastUpdated, refresh }`;`seriesTick(ts: number, spanMs: number): string`。

- [ ] **Step 1: 写失败的测试(composable)**

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { useOverview } from '@/composables/useOverview'
import { relativeDraft, emptyDraft } from '@/lib/failureFilter'

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('fetch', vi.fn())
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('useOverview', () => {
  it('stops polling once the draft is an absolute bracket', async () => {
    // D8: a week refreshed every 15s is a load test, not a live view.
    const draft = ref({ ...emptyDraft(), fromMs: 1_760_000_000_000, toMs: 1_760_600_000_000 })
    const { refresh } = useOverview(draft)
    await refresh()
    const calls = vi.mocked(fetch).mock.calls.length
    await vi.advanceTimersByTimeAsync(45_000)
    expect(vi.mocked(fetch).mock.calls.length).toBe(calls)
  })

  it('keeps polling a relative window', async () => {
    const draft = ref(relativeDraft(60))
    useOverview(draft)
    await vi.advanceTimersByTimeAsync(45_000)
    expect(vi.mocked(fetch).mock.calls.length).toBeGreaterThan(1)
  })

  it('refuses a bad bracket without asking the server', async () => {
    const draft = ref({ ...emptyDraft(), windowMinutes: 20_000 })
    const { invalid, refresh } = useOverview(draft)
    await refresh()
    expect(vi.mocked(fetch)).not.toHaveBeenCalled()
    expect(invalid.value).toContain('10080')
  })

  it('drops the previous bracket\'s data when the new one is refused', async () => {
    const draft = ref(relativeDraft(60))
    const { data, invalid, refresh } = useOverview(draft)
    vi.mocked(fetch).mockResolvedValueOnce(
      new Response(JSON.stringify({ windowMinutes: 60, series: [], instancesOnline: 0, instancesTotal: 0, queries: 1, failures: 0, successRate: 1, p95LatencyMs: null }), { headers: { 'content-type': 'application/json' } }),
    )
    await refresh()
    expect(data.value).not.toBeNull()
    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await refresh()
    // A chart that no longer matches the URL is worse than an empty one.
    expect(data.value).toBeNull()
    expect(invalid.value).toContain('10080')
  })
})
```

> 先读 `doggy-dns-ui/src/api/http.ts` 确认 `apiGet` 走的是 `fetch`(若是 axios 之类,把 spy 换成该层的实际出口)。`/failures` 侧的 `useFailureList` 已有同类测试可参考 `src/composables/__tests__/`。

- [ ] **Step 2: 跑测试确认失败**

Run: `bun x vitest run src/composables/__tests__/useOverview.spec.ts`
Expected: FAIL(现在 `useOverview(initialWindow: number)` 根本没有 draft 参数)。

- [ ] **Step 3: 改 composable**

```ts
import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { ApiError, apiGet } from '@/api/http'
import type { Overview } from '@/api/types'
import {
  isAbsolute,
  resolveBracket,
  toRequestQuery,
  validateBracket,
  validateDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

const POLL_INTERVAL_MS = 15_000

function buildQuery(params: Record<string, string>): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) search.set(key, value)
  return search.toString()
}

export function useOverview(draft: Ref<FilterDraft>) {
  const data = ref<Overview | null>(null)
  const error = ref<ApiError | null>(null)
  const invalid = ref<string | null>(null)
  const loading = ref(false)
  const lastUpdated = ref<number | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let seq = 0

  async function refresh() {
    const requestSeq = ++seq
    const now = Date.now()
    const refusal = validateDraft(draft.value) ?? validateBracket(resolveBracket(draft.value, now), now)
    if (refusal) {
      // The URL asks something illegal: say so, show nothing stale behind it.
      invalid.value = refusal
      data.value = null
      error.value = null
      return
    }
    invalid.value = null
    loading.value = true
    try {
      const fetched = await apiGet<Overview>(`/api/overview?${buildQuery(toRequestQuery(draft.value))}`)
      if (requestSeq !== seq) return
      data.value = fetched
      error.value = null
      lastUpdated.value = Date.now()
    } catch (cause) {
      if (requestSeq !== seq) return
      error.value = cause instanceof ApiError ? cause : new ApiError(0, 'network', String(cause))
    } finally {
      if (requestSeq === seq) loading.value = false
    }
  }

  function restartTimer() {
    if (timer) {
      clearInterval(timer)
      timer = null
    }
    if (!isAbsolute(draft.value)) timer = setInterval(() => void refresh(), POLL_INTERVAL_MS)
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

  return { data, error, invalid, loading, lastUpdated, refresh }
}
```

- [ ] **Step 4: 改视图**

`OverviewView.vue`:

1. 删 `windows` 数组与胶囊组,header 改成 `<TimeRangeControl :model-value="draft" @update:model-value="setDraft" />`(与刷新按钮并排);`useOverview(60)` → `useOverview(draft)`。
2. draft 由路由读写,`/failures` 页同款:

```ts
const route = useRoute()
const router = useRouter()
const draft = computed(() => parseFilterQuery(route.query))

function setDraft(next: FilterDraft) {
  void router.replace({ query: toRouteQuery(next) })
}
```

初始化:URL 无参数时 `parseFilterQuery` 给的是全 `null` draft,请求会落到服务端默认 60——但为了 `aria-pressed` 有归属,`onMounted` 时若 `route.query` 里没有 `from`/`to`/`windowMinutes`,用 `router.replace({ query: toRouteQuery(relativeDraft(DEFAULT_WINDOW_MINUTES)) })` 补上(与 Task 3 的默认值同源于 `DEFAULT_WINDOW_MINUTES`,不写死 60)。
3. 区间标签进 header:`<p class="mt-1 text-sm text-muted">{{ rangeLabel(draft, Date.now()) }}</p>`(绝对区间显示 `起 ~ 止`,相对显示"近 N 小时"),`Queries` 卡片的 `hint="selected window"` 改成 `:hint="rangeLabel(draft, Date.now())"`。
4. p95 卡片改口径文案(D16 的代价必须写在脸上):

```html
<KpiCard
  label="P95 latency"
  :value="data?.p95LatencyMs ?? null"
  format="ms"
  hint="worst bucket in range"
/>
```

5. 错误横幅:`invalid` 优先于 `error`,且 refusal 时不显示"展示的是上次成功数据"(数据已被清空):

```ts
const errorMessage = computed(() => {
  if (invalid.value) return invalid.value
  if (!error.value) return ''
  if (data.value) return 'OpenObserve 暂不可用，展示的是上次成功数据'
  return '无法加载 Overview，请检查 OpenObserve 连接'
})
```
模板 `<ErrorBanner v-if="error || invalid" …>`。

- [ ] **Step 5: 趋势轴按跨度出标签**

Create `doggy-dns-ui/src/lib/chartLabels.ts`:

```ts
/**
 * An x-axis tick. A one-hour view has always read `HH:mm`; a week of the same
 * labels would be seven identical columns, so anything spanning more than a day
 * gets the date too. The threshold is about the labels, not the bucket width.
 */
export function seriesTick(ts: number, spanMs: number): string {
  const date = new Date(ts)
  if (spanMs <= 24 * 60 * 60 * 1000) {
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  }
  return `${date.toLocaleDateString([], { month: '2-digit', day: '2-digit' })} ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
}

/// The span the points cover, in milliseconds; one point or none is a day so
/// the short label wins.
export function seriesSpanMs(points: { ts: number }[]): number {
  if (points.length < 2) return 0
  return points[points.length - 1].ts - points[0].ts
}
```

(上面第二条注释写成 `/** */`,不是 `///`。)

`TrendChart.vue` 的 `xAxis.data` 改成:

```ts
data: (() => {
  const spanMs = seriesSpanMs(props.series)
  return props.series.map((point) => seriesTick(point.ts, spanMs))
})(),
```

`tests`:`bun x vitest run src/lib/__tests__/chartLabels.spec.ts` 新建两条用例——`seriesTick(ts, 60 * 60 * 1000)` 不含日期、`seriesTick(ts, 7 * 24 * 3600 * 1000)` 含 `month/day`(用 `vi.setSystemTime` + `TZ` 固定,或直接用本地时区构造 `new Date(2026, 0, 1, 3, 0)` 避免时区漂移)。

- [ ] **Step 6: 更新 OverviewView 的既有用例**

`src/views/__tests__/OverviewView.spec.ts`:原来点 `6h` 胶囊的地方改为点 `[data-testid="quick-6h"]`;新增两条:
- `/?windowMinutes=10080` 深链 → fetch 出去的 URL 含 `windowMinutes=10080`,且不发第二个请求(轮询停)。
- `?from&to` 深链 → header 出现 `起 ~ 止` 形态的区间标签。

Run: `bun x vitest run src/views/__tests__/OverviewView.spec.ts src/composables/__tests__/useOverview.spec.ts src/lib/__tests__/chartLabels.spec.ts`
Expected: PASS。

- [ ] **Step 7: 红/绿自查 + gate + 提交**

把 `restartTimer` 的 `isAbsolute` 判断去掉,确认"stops polling"用例变红;改回。
Run: `bun run type-check && bun run lint && bun run test:unit -- --run`

```bash
git add doggy-dns-ui/src
git commit -m "$(cat <<'EOF'
feat: let the overview take a week-long bracket

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 7: 实例抽屉的时间控件与区间内失败数

**Files:**
- Modify: `doggy-dns-ui/src/composables/useInstances.ts`
- Modify: `doggy-dns-ui/src/api/types.ts`(`recentFailures` → `failureCount: number | null`)
- Modify: `doggy-dns-ui/src/views/InstancesView.vue`
- Modify: `doggy-dns-ui/src/views/__tests__/InstancesView.spec.ts`
- Modify: `doggy-dns-ui/e2e/console.spec.ts`

**Interfaces:**
- Consumes: Task 5 的 `TimeRangeControl`、Task 4 的 `/api/instances/{id}?from&to|windowMinutes`。
- Produces: `useInstanceDetail(id: Ref<string|null>, draft: Ref<FilterDraft>)`;列表请求 `/api/instances` **不带任何时间参数**(它是唯一没有区间语义的端点)。

- [ ] **Step 1: 写失败的测试**

```ts
it('re-requests the detail when the bracket changes', async () => {
  // The drawer's number is the one thing on this page that follows the range.
  const draft = ref(relativeDraft(60))
  const { refresh } = useInstanceDetail(ref('dns-node-1'), draft)
  await refresh()
  expect(fetchUrl(0)).toContain('/api/instances/dns-node-1?windowMinutes=60')
  draft.value = { ...emptyDraft(), fromMs: 1_760_000_000_000, toMs: 1_760_600_000_000 }
  await refresh()
  expect(fetchUrl(1)).toContain('from=1760000000000')
  expect(fetchUrl(1)).toContain('to=1760600000000')
  expect(fetchUrl(1)).not.toContain('windowMinutes')
})

it('renders an em dash when telemetry could not count', async () => {
  const wrapper = await mountDrawer({ failureCount: null })
  expect(wrapper.text()).toContain('—')
  expect(wrapper.text()).not.toMatch(/区间内失败\s*0/)
})
```

(具体 spy / mount helper 照 `src/views/__tests__/InstancesView.spec.ts` 现有写法,那里已经有 stub 详情对象的先例,`recentFailures: 0` 就在第 58 行——改成 `failureCount: 0`,并另加一条 `failureCount: null` 的用例。)

- [ ] **Step 2: 跑测试确认失败**

Run: `bun x vitest run src/views/__tests__/InstancesView.spec.ts`
Expected: FAIL。

- [ ] **Step 3: 实现**

`useInstances.ts`:

```ts
export function useInstanceDetail(id: Ref<string | null>, draft: Ref<FilterDraft>) {
  ...
  async function refresh() {
    if (!id.value) { detail.value = null; return }
    loading.value = true
    try {
      // The bracket travels with the request: the failure number is a range
      // answer, everything else in the detail is live MCP data.
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      detail.value = await apiGet<InstanceDetail>(
        `/api/instances/${encodeURIComponent(id.value)}?${query}`,
      )
      ...
  }
  watch([id, draft], () => void refresh(), { immediate: true })
}
```

`InstancesView.vue`:页面级 draft(`parseFilterQuery(route.query)` + `setDraft` → `router.replace`,与 Task 6 Step 4 完全同款);`<TimeRangeControl>` 挂在 header 里(与 Refresh 并排);详情抽屉的指标组加一张卡:

```html
<div class="panel p-3">
  <p class="text-[10px] uppercase tracking-wide text-muted">区间内失败</p>
  <p class="mt-1 text-sm">{{ dash(detail.failureCount) }}</p>
  <RouterLink
    class="mono mt-1 inline-block text-[10px] text-accent"
    :to="{ name: 'failures-list', query: { ...toRouteQuery(draft), instance: detail.instance.host } }"
  >在失败列表里看这台主机</RouterLink>
</div>
```

`dash(...)` 若该组件没有就用 `detail.failureCount ?? '—'`。列表请求保持 `apiGet<Instance[]>('/api/instances')` 不变。

- [ ] **Step 4: e2e 钉住"详情真的按区间取数"**

`doggy-dns-ui/e2e/console.spec.ts` 追加(demo 模式,fixture 的分组铺到 8 天前):

```ts
test('the instance drawer counts failures over the chosen bracket', async ({ page }) => {
  await page.goto('/instances?windowMinutes=10080')
  await page.getByRole('button', { name: /dns-node-1/ }).first().click()
  const drawer = page.getByRole('dialog', { name: 'Instance detail' })
  await expect(drawer.getByText('区间内失败')).toBeVisible()
  const week = await drawer.locator('section').first().innerText()
  await page.goto('/instances?windowMinutes=60')
  await page.getByRole('button', { name: /dns-node-1/ }).first().click()
  const hour = await page.getByRole('dialog', { name: 'Instance detail' }).locator('section').first().innerText()
  expect(week).not.toEqual(hour)
})
```

(e2e 里定位数字的选择器请按实现后的 DOM 收窄,别用 `.first()` 猜——目标是"两次文本不同",不是"抓到某个 number"。)

- [ ] **Step 5: 红/绿自查 + gate + 提交**

把 `useInstanceDetail` 的请求 URL 改回不带 query,确认 Step 1 的用例变红;改回。
Run: `bun run type-check && bun run lint && bun run test:unit -- --run && bun run test:e2e`

```bash
git add doggy-dns-ui/src doggy-dns-ui/e2e
git commit -m "$(cat <<'EOF'
feat: scope the instance drawer's failure count to the chosen bracket

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

### Task 8: 文档、端到端复核与全量 gate

**Files:**
- Modify: `README.md`(`:102-107` 的时间段、`:134` 的错误段、`:273` 的深链段)
- Modify: `docs/superpowers/plans/2026-11-03-absolute-range-failure-list.md`(在该计划末尾加一行"看板/实例区间由 2026-11-04 计划接管")
- Modify: `doggy-dns-ui/e2e/console.spec.ts`(看板 7 天用例)

- [ ] **Step 1: README 改口径**

`/api/overview` 一行写成 `GET /api/overview?from=&to=&windowMinutes=`,并把第 102–107 行那段"the live class … clamped to 5–1440"整段替换:所有取区间的端点(overview、instances/{id}、failures 全家)共用 5–10080 与同一套 400 文案,`/api/instances` 列表不取时间窗。新增两句:p95 是"区间内最差分桶"(桶宽 = `ceil(span/168)` 分钟,夹在 1–60,默认 1 小时就是 5 分钟 rate 窗口 ×60s 步长),趋势线在 7 天区间是每小时一个点。实例详情补 `failureCount` 的语义:`COUNT(*)` 不封顶,`null` 表示遥测算不出来(不是 0)。

Run: `grep -n "1440\|clamp" README.md`
Expected: 仅剩真正与 clamp 无关的历史描述(应当为空或只剩 `Window` 内部用途那一句)。

- [ ] **Step 2: 看板 7 天 e2e**

```ts
test('the overview takes a week and stops refreshing', async ({ page }) => {
  await page.goto('/?windowMinutes=10080')
  const request = page.waitForResponse(/\/api\/overview\?windowMinutes=10080$/)
  await request
  // D8: the absolute/relative distinction is about the preset, so a relative
  // week still refreshes; assert the refusal path instead for the fixed case.
  await page.goto('/?from=1760000000000&to=1760600000000')
  await expect(page.getByTestId('range-hint')).toBeVisible()
})
```

(第二条要真正钉"绝对区间不再每 15s 请求":数 20s 内 `/api/overview` 响应次数 == 1。用 `page.on('response')` 计数,别用 `waitForResponse` 猜。)

- [ ] **Step 3: 浏览器实测(不是"看起来对")**

demo 模式起服务:`CONSOLE_DEMO=1 bun run dev`(或后端直读 `dist`:先 `cd doggy-dns-ui && bun run build`,再在仓库根 `CONSOLE_DEMO=1 cargo run`),用 chrome-devtools 走:
1. `/` → 点"7 天" → 网络里 `/api/overview?windowMinutes=10080`,趋势轴出现日期;等 20s 看是否仍刷新(相对窗口应当继续刷新,记录次数)。
2. `/` → 手输 `?from&to` 绝对区间 → 20s 内只 1 次请求。
3. `/` → `?windowMinutes=20000` → 卡片全空 + 横幅文案含"10080"。
4. `/instances` → 选 7 天 → 打开 dns-node-1 → "区间内失败" 数字随区间变;`failureCount` 为 `null` 时显示 `—`(可临时在 devtools 里改 `$pinia`/组件状态验证,或直接看 `/api/instances/dns-node-1` 的 JSON 形状)。
5. `/failures` → 三处时间控件形态一致、预设 1h/6h/24h/7d 都在。
6. p95 卡片 hint 是 "worst bucket in range"。
把实测到的数字/文案原样贴回报告,不许只写"符合预期"。

- [ ] **Step 4: 全量 gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Run: `cd doggy-dns-ui && bun run type-check && bun run lint && bun run test:unit -- --run && bun run test:e2e`
Expected: 全绿;把每条命令的 exit code 与测试计数原样记录(不得凭记忆写数字)。

- [ ] **Step 5: 提交**

```bash
git add README.md docs doggy-dns-ui/e2e
git commit -m "$(cat <<'EOF'
docs: state the one range contract, the bucketed p95 and the live count

Co-authored-by: Qoder <noreply@qoder.com>
EOF
)"
```

---

## Self-Review 记录

- **Spec 覆盖**:D2 → Task 3/4;D8 → Task 6(轮询)+ Task 2(总量单点);D9 → Task 3/4(`get_overview`/`get_instance_detail` 参数与描述);D15 → Task 1/2;D16 → Task 2(worst bucket)+ Task 6(卡片 hint)+ Task 8(README);D17 → Task 4(不封顶 `COUNT(*)`、`Option`)+ Task 7(可见数字、跳列表、URL 同步);§6.5/6.6 → Task 5/6;§7 全部新钉子分布在 Task 1–8。
- **占位符扫描**:无 TBD;"按现有 helper 形状改写"的三处(`tests/overview_test.rs` 的响应体与 telemetry 构造、`tests/api_test.rs` 的 get helper、e2e 的抽屉定位器)都指明了参照文件与行号,属于对既有形状的显式引用,不是未定义内容。
- **类型一致性**:`Range::bucket_minutes/rate_window_minutes/step_seconds/whole_step_seconds/span_minutes`(Task 1 定义,Task 2/4 消费)、`RangeQuery::range(now_ms)`(Task 3 定义,Task 4 消费)、`failureCount: number | null`(Task 4 后端 `Option<u64>` ↔ Task 7 TS)、`useOverview(draft: Ref<FilterDraft>)`(Task 6 定义,同任务消费)已逐处对齐。
