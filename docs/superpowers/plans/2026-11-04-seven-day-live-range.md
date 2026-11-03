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
- Modify: `src/datasource/mcp.rs:307`、`src/datasource/instances.rs:397`、`src/datasource/fixture.rs:398`(三处 `recent_failures: 0` 构造点;用 `grep -rn "recent_failures" src tests` 定位,别信行号)
- Modify: `src/service.rs`(`instance_detail(id, range)`;改完 `Window` 在该文件只剩 import,删掉它,否则 `clippy -D warnings` 红)
- Modify: `src/api/instances.rs`、`src/agent/tools.rs`(`InstanceArgs` + `get_instance_detail` + 描述)
- Modify: `tests/fixture_test.rs:97`、`tests/instances_test.rs`、`tests/tools_test.rs:41`、`tests/api_test.rs:125`、`tests/evidence_test.rs:461`(`fn instance_detail()` helper,字段在 `:494`)

**Interfaces:**
- Consumes: Task 3 的 `crate::api::RangeQuery`。
- Produces: `async fn instance_failure_count(&self, filter: FailureFilter) -> Result<u64, DataError>`;`InstanceDetail.failure_count: Option<u64>`(JSON `failureCount`,计不到数为 `null`);`ConsoleService::instance_detail(&self, id: &str, range: Range)`。

- [ ] **Step 1: 写失败的测试(SQL 文本)**

在 `tests/failures_test.rs` 追加(该文件 `:5-7` 已经 `use doggy_dns_console::datasource::openobserve::failures::{failure_group_query, …, unlabeled_failures_query};`,把 `instance_failure_count_query` 加进那个列表;`:14` 已经 import `FailureFilter` 与 `Range`):

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

`src/datasource/openobserve/telemetry.rs` 实现它,照 `unlabeled_failures`(:158-175)的形状——同一条 `search_logs(&sql, start, end, 1)`、同一个 `rows.iter().find_map(|row| row.get("count").and_then(Value::as_i64))`。取值成功就 `Ok(count.max(0) as u64)`。**取不到整数那一支不照抄**:那里是 `warn + Ok(0)`,本期不能沿用(见下)。

`src/datasource/fixture.rs` 实现:`self.groups()` 里 `self.matches(group, &filter)` 为真的 `count` 之和(`matches` 已含区间、domain、responseCode 与 host 等值:`src/datasource/fixture.rs:141-169`)。

两条口径决定(照做,别自己再判一遍):
- **COUNT 查不到整数那一支返回 `Err`,不是 `Ok(0)`。** `unlabeled_failures` 报 0 是对的——"没有未标记行"本身是答案;而这里 0 会被抽屉读成"这周这台节点很健康",正是 D17 禁止的发明。写法:`tracing::warn!` 后 `Err(DataError::Internal("instance failure count query returned no integer count"))`,让 service 那一侧落到 `None`。
- **不要为省事把 `instance_failure_count_query` 与 `unlabeled_failures_query` 合成一个带开关的函数。** 两者各是一行 `format!`,差别在谓词(host 等值 vs host 为空)而不是在形状;两个具名函数在调用点可读,`unlabeled_failures_query` 的 doc comment 已经写明"dropping the host equality is the point",混成一个就没人再读得到这句。

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
        // 这个数是一条 host 等值查询,所以只有"这台实例真有 host 标签"才有意义。
        // host 为空时 filter 会退化成"不加实例条件",把整支队的失败算到一台头上。
        let Some(host) = host_filter(Some(detail.instance.host.clone())) else {
            tracing::warn!(
                instance = %detail.instance.id,
                "instance has no host label; a host-equality count would be fleet-wide, reporting no number"
            );
            return Ok(detail);
        };
        let filter = FailureFilter {
            instance: Some(host),
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

`src/service.rs` 的 import 要跟着动:`host_filter` 现在**不在**该文件的 `use crate::domain::{...}` 里(`src/service.rs:4-6`),加进去;`Window` 在第 89 行那处 `Range::from_window(Window::new(60), …)` 被删掉后只剩 import,一并删——`clippy -D warnings` 会替你把这两条都抓出来。

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

`src/agent/tools.rs`:`InstanceArgs`(`:59-62`,derive 是 `Debug, Clone, Serialize, Deserialize, JsonSchema` + `rename_all = "camelCase"`,**没有 `Default`**)加 `window_minutes`/`from`/`to` 三字段(doc comment 照 `OverviewArgs` 那三句抄),derive 列表保持不动——不要为了让测试少写两行而给它加 `Default`:`instance_id: String` 默认成空串是个没意义的值。`get_instance_detail` 用 `Range::resolve` 解析后传给 service,并把该工具的 description 补一句 `Pass the same from/to or windowMinutes the operator is looking at.`

`tests/tools_test.rs:41-48` 那条既有用例用的是**完备字面量** `InstanceArgs { instance_id: ... }`,补三个 `None` 字段而不是 `..Default::default()`。

- [ ] **Step 5: 更新既有测试**

- `tests/fixture_test.rs:62` `fixture_instance_detail_counts_recent_failures` 改名 `fixture_instance_detail_counts_failures_in_the_bracket`,断言:1 小时区间(`bracket(60)` 现成 helper)下 `failure_count == Some(n)` 且 7 天区间(`bracket(10_080)`)下 `7d > 1h` —— 这条钉子证明 fixture 真的按区间取数,而不是抄一个常量。
- `tests/evidence_test.rs:461` `instance_detail()` 构造体加 `failure_count: None`。
- `tests/instances_test.rs`、`tests/tools_test.rs:42`、`tests/api_test.rs` 里所有 `instance_detail(` 调用补区间参数。
- `tests/instances_test.rs` 加一条:数据源返回 `Err` 时 `failure_count` 是 `None`(用 `SeenOnly` 那个 stub 的同族写法),钉住"不发明 0"。再加一条:host 为空的实例走 `instance_detail` 时同样是 `None`,并且**不许**发出一次没有 host 条件的 count 查询(用一个记录 filter 的 stub 断言它压根没被调用,或断言收到的 filter 里 `instance` 不是 `None`)。
- `tests/api_test.rs` 加两条:
  `/api/instances/dns-node-1?from={}&to={}`(1 小时,`FIXED_NOW_MS` 同款固定时钟)返回 200 且 body 有 `failureCount`;
  `/api/instances/dns-node-1?windowMinutes=20000` 返回 400 且 `error.code == "invalid_request"`——与 `/api/overview`、`/api/failures` 同一句文案(直接用 Step 2 那条"两端同文案"的写法扩成三端)。
- `tests/api_test.rs` 再加一条钉住"列表端点不取区间"这个口径:`/api/instances?from=1&to=2` 仍然 200(多余参数被忽略)。`/api/instances` 是本期唯一没有区间语义的端点,这条钉子防止后来者"顺手"给它加 `Query<RangeQuery>`。

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
- Create: `doggy-dns-ui/src/components/__tests__/TimeRangeControl.spec.ts`
- Modify: `doggy-dns-ui/src/components/FailureFilterBar.vue`
- Modify: `doggy-dns-ui/src/components/__tests__/FailureFilterBar.spec.ts`(预设清单补 6h)
- Modify: `doggy-dns-ui/src/views/__tests__/FailuresView.spec.ts`(:274-280 钉着已退役的"看板仍限 1440"文案,必须跟着反转)
- Modify: `doggy-dns-ui/src/lib/failureFilter.ts:198`、`src/lib/__tests__/failureFilter.spec.ts:225`(**只改注释**:两处都写"1440 是实时看板最宽的选项",Task 5 之后不成立。`rangeLabel` 的行为不动 —— 1440 仍读"近 24 小时"、10080 读"近 7 天",这条翻转只是分单位,不是上限)

`src/views/FailuresView.vue` 不改:它只渲染 `<FailureFilterBar>`,而 `mount` 深渲染子组件,所以 spec 里 `find('[data-testid="range-hint"]')` / `quick-7d` 在抽取后仍然命中。同理 `FailureFilterBar.spec.ts:20-24` 的 `dateInput()`(按 `input[type="datetime-local"]` 的位置找)穿过子组件照样有效,不必改。

**Interfaces:**
- Consumes: `@/lib/failureFilter` 的 `FilterDraft`/`relativeDraft`/`datetimeToMs`/`msToDatetime`/`DEFAULT_WINDOW_MINUTES`/`MIN_RANGE_MINUTES`/`MAX_RANGE_MINUTES`(**不改这个模块的行为**)。
- Produces: `<TimeRangeControl :model-value="draft" @update:model-value="..." />` —— 只负责时间那一段(预设 + 起/止 + 回到近 1 小时 + 口径提示),不碰 domain/responseCode/instance。Task 6、Task 7 各挂一个。

- [ ] **Step 1: 写失败的测试**

```ts
import { mount, type VueWrapper } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import TimeRangeControl from '@/components/TimeRangeControl.vue'
import { emptyDraft, msToDatetime, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000

// `VueWrapper`, the same handle `FailureFilterBar.spec.ts` uses: `mount` is
// generic, so `ReturnType<typeof mount>` collapses to a wrapper the helpers
// cannot index.
function hintText(wrapper: VueWrapper): string {
  return wrapper.get('[data-testid="range-hint"]').text()
}

function lastDraft(wrapper: VueWrapper): FilterDraft {
  const events = wrapper.emitted('update:modelValue')
  if (!events || events.length === 0) throw new Error('the control emitted no draft')
  return events[events.length - 1]![0] as FilterDraft
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
    expect(lastDraft(wrapper)).toMatchObject({ fromMs: null, toMs: null, windowMinutes: 10080 })
  })

  it('never emits a half bracket', async () => {
    const wrapper = mount(TimeRangeControl, { props: { modelValue: emptyDraft() } })
    // An hour ago, not a fixed 2026-01-01: `setFrom` completes the pair with the
    // browser's now, so a dated-from would make the emitted bracket wider than
    // the very ceiling the hint below advertises — the test would pass while
    // proving nothing about a draft the page could actually submit.
    const fromText = msToDatetime(Date.now() - HOUR)

    await wrapper.get('[data-testid="range-from"]').setValue(fromText)

    const draft = lastDraft(wrapper)
    expect(draft.fromMs).toBe(Date.parse(fromText))
    expect(draft.toMs).not.toBeNull()
    expect(draft.windowMinutes).toBeNull()
  })

  it('states the one ceiling and the one endpoint without a window', () => {
    const hint = hintText(mount(TimeRangeControl, { props: { modelValue: emptyDraft() } }))
    expect(hint).toContain('5 分钟到 10080 分钟')
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
    5–10080 minutes instead of clamping (`Range::resolve` in `src/domain.rs`,
    reached from `RangeQuery::range`, `FailureQuery` and each copilot tool), so
    the numbers below are the request contract, not a per-page ceiling. The one
    remaining `Window::new` clamp lives in internal host discovery
    (`src/datasource/instances.rs`), which no request parameter reaches.
    `/api/instances` is the one endpoint with no window at all — its rows are
    identity and reachability, so the hint has to name it or an operator will
    read a frozen instance list as a stale one.
    注释里写死数字、正文插值:模板注释不参与编译,别在这里放 mustache。
  -->
  时间区间支持 {{ MIN_RANGE_MINUTES }} 分钟到 {{ MAX_RANGE_MINUTES }} 分钟（7 天），所有端点超限直接拒绝、不再夹回；
  实例列表（/api/instances）不取时间窗。
</p>
```

- [ ] **Step 3: 让筛选条只用它,并反转两处旧断言**

`FailureFilterBar.vue` 删掉搬走的片段与 `QUICK_WINDOWS`,改成 `<TimeRangeControl :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" />`,其余(实例/响应码/域名)不动。

然后**必须**改这两处已经变成假话的断言(Step 1 的新文案让旧断言必红;不反转就等于把旧文案钉回来):

- `src/views/__tests__/FailuresView.spec.ts:274-280` —— 用例名 `quotes both ceilings in the hint line` 与 `expect(hint.text()).toContain('1440')`。改名 `quotes the one ceiling and names the windowless endpoint`,把 `toContain('1440')` 换成 `expect(hint.text()).not.toContain('1440')` + `expect(hint.text()).toContain('/api/instances')`;`toContain('5 分钟到 10080 分钟')` 那行留着,它是这条文案唯一还成立的数字断言。
- `src/components/__tests__/FailureFilterBar.spec.ts:122` —— 循环清单 `['quick-1h', 'quick-24h', 'quick-7d']` 补 `'quick-6h'`。新增预设不补进"绝对区间时谁都不许亮"的清单,等于没测。

顺带把 Files 里那两处"1440 是最宽选项"的注释改成实话(不碰任何行为)。

Run: `bun x vitest run src/components/__tests__/FailureFilterBar.spec.ts src/views/__tests__/FailuresView.spec.ts`
Expected: PASS。

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
- Modify: `doggy-dns-ui/src/composables/useOverview.ts`(`useOverview(initialWindow = 60)` → `useOverview(draft: Ref<FilterDraft>)`;只 `OverviewView.vue:14` 一个调用点)
- Create: `doggy-dns-ui/src/composables/__tests__/useOverview.spec.ts`(此文件现在不存在,Step 1 是新建)
- Modify: `doggy-dns-ui/src/views/OverviewView.vue`
- Create: `doggy-dns-ui/src/lib/chartLabels.ts`、`doggy-dns-ui/src/lib/__tests__/chartLabels.spec.ts`
- Modify: `doggy-dns-ui/src/components/TrendChart.vue`(只换 `xAxis.data` 的标签来源)
- Modify: `doggy-dns-ui/src/views/__tests__/OverviewView.spec.ts`

`src/components/__tests__/KpiCard.spec.ts` **不改**:`hint` 早就是 `KpiCard` 的入参,本任务只换传进去的字符串。`e2e/console.spec.ts` 也不改——它没点过看板的任何时间胶囊(全文只 `goto('/')` + `/failures?...`)。

**Interfaces:**
- Consumes: Task 5 的 `TimeRangeControl`;`failureFilter.ts` 的 `parseFilterQuery`/`toRouteQuery`/`toRequestQuery`/`isAbsolute`/`validateDraft`/`validateBracket`/`resolveBracket`/`rangeLabel`/`relativeDraft`/`DEFAULT_WINDOW_MINUTES`(**不改这个模块的行为**);Task 3 的 `/api/overview?from&to`。
- Produces: `useOverview(draft: Ref<FilterDraft>)` → `{ data, error, invalid, loading, lastUpdated, refresh }`(不再有 `windowMinutes`/`setWindow`);`seriesTick(ts: number, spanMs: number): string`、`seriesSpanMs(points: { ts: number }[]): number`。

- [ ] **Step 1: 写失败的测试(composable)**

`apiGet` 走的就是全局 `fetch`(`src/api/http.ts:13`),所以 spy `fetch` 是对的出口。

**必须**照 `useFailureList.spec.ts:120-135` 的宿主组件写法挂载:`useOverview` 的首次请求和轮询都登记在 `onMounted` 里,在 setup 上下文之外直接调用 `useOverview(draft)` 时 `onMounted` 是空操作——那样"停轮询"用例会因为**什么都没启动**而通过,等于没测。

```ts
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useOverview } from '@/composables/useOverview'
import { emptyDraft, relativeDraft, type FilterDraft } from '@/lib/failureFilter'

const HOUR = 3_600_000
const NOW = 1_761_436_800_000

const overviewBody = {
  windowMinutes: 60,
  instancesOnline: 3,
  instancesTotal: 4,
  queries: 10,
  failures: 1,
  successRate: 0.9,
  p95LatencyMs: 12.4,
  series: [],
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

const wrappers: VueWrapper[] = []
let fetches: string[] = []

// 照 `useFailureList.spec.ts` 的 `mountList`:helper 里不 await,`mount()` 已同步
// 跑完 `onMounted`,请求落地由用例自己 `await flushPromises()`。
function mountOverview(draft: Ref<FilterDraft>, respond: () => Response = () => json(overviewBody)) {
  fetches = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      fetches.push(String(input))
      return respond()
    }),
  )
  let overview!: ReturnType<typeof useOverview>
  const wrapper = mount(
    defineComponent({
      setup() {
        overview = useOverview(draft)
        return () => h('div')
      },
    }),
  )
  wrappers.push(wrapper)
  return { overview, wrapper }
}
```

```ts
beforeEach(() => {
  vi.useFakeTimers()
  // 钉住时钟:`setFrom` 之类的 `Date.now()` 与 15 s 轮询都要落在可预测的刻度上。
  vi.setSystemTime(NOW)
})
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('useOverview', () => {
  it('asks for the absolute bracket and stops polling it', async () => {
    // D8: a week refreshed every 15 s is a load test, not a live view.
    const draft = ref({ ...emptyDraft(), fromMs: NOW - 3 * HOUR, toMs: NOW })
    const { wrapper } = mountOverview(draft)
    await flushPromises()

    expect(fetches.length).toBe(1)
    expect(fetches[0]).toContain(`from=${NOW - 3 * HOUR}`)
    expect(fetches[0]).toContain(`to=${NOW}`)
    expect(fetches[0]).not.toContain('windowMinutes=')

    const initial = fetches.length
    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(initial)
    wrapper.unmount()
  })

  it('keeps polling a relative window', async () => {
    const { wrapper } = mountOverview(ref(relativeDraft(60)))
    await flushPromises()
    expect(fetches.length).toBe(1)

    await vi.advanceTimersByTimeAsync(45_000)
    expect(fetches.length).toBe(4) // 15 s × 3
    wrapper.unmount()
  })

  it('refuses a bad bracket without asking the server', async () => {
    const { overview, wrapper } = mountOverview(ref({ ...emptyDraft(), windowMinutes: 20_000 }))
    await flushPromises()

    expect(fetches.length).toBe(0)
    expect(overview.invalid.value).toContain('10080')
    expect(overview.error.value).toBeNull()
    wrapper.unmount()
  })

  it('drops the previous bracket\'s data when the new one is refused', async () => {
    const draft = ref(relativeDraft(60))
    const { overview, wrapper } = mountOverview(draft)
    await flushPromises()
    expect(overview.data.value).not.toBeNull()

    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await flushPromises()
    // A chart that no longer matches the URL is worse than an empty one.
    expect(overview.data.value).toBeNull()
    expect(overview.invalid.value).toContain('10080')
    wrapper.unmount()
  })

  it('retires an in-flight answer when the bracket that asked for it is refused', async () => {
    // `refresh()` bumps the sequence before it looks at the draft, so the
    // answer still in flight belongs to a bracket that no longer exists.
    // Without that, the late response would re-paint a chart the URL refuses.
    let release!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => {
      release = resolve
    })
    const draft = ref(relativeDraft(60))
    const { overview, wrapper } = mountOverview(draft, () => pending)
    await flushPromises()

    draft.value = { ...emptyDraft(), windowMinutes: 20_000 }
    await flushPromises()
    expect(overview.data.value).toBeNull()

    release(json(overviewBody))
    await flushPromises()
    expect(overview.data.value).toBeNull()
    wrapper.unmount()
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun x vitest run src/composables/__tests__/useOverview.spec.ts`
Expected: FAIL。vitest 不做类型检查,所以是**行为**变红:现在的 `useOverview(initialWindow = 60)` 把传进来的 `Ref<FilterDraft>` 当成窗口数字,首个请求是 `?windowMinutes=[object Object]`,既没有 `from=`/`to=`("asks for the absolute bracket" 红),也照样发了请求("refuses a bad bracket" 期望 0 次,实到 1 次)。`bun run type-check` 在同一处报签名不匹配——两边都红才算这条用例真的钉住了新签名。

- [ ] **Step 3: 改 composable**

两处口径先说清,review 时不必再猜:

1. **不复制 `buildQuery`。** `useFailureList.ts:18-22` 有一个私有 `buildQuery`,但这里只需要一行 `new URLSearchParams(...)`;把它抄进第二个文件才是评审会点的重复。直接用标准库,`toRequestQuery` 的返回值就是它要的 `Record<string, string>`。
2. **`invalid` 与 `error` 分成两个 ref,和 `useFailureList` 的单一 `error` 不一样,是有意为之**:`useFailureList` 只把消息原样贴进一行提示;看板要把"你给的区间不合法"和"OpenObserve 挂了"分开来讲——后者的文案是"展示的是上次成功数据",而拒绝路径下数据已被清空,套同一句就是假话。`ApiError` 类型的 `error` 保持原样(视图按 `status` 组织文案),所以拒绝必须有另一个出口。

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

export function useOverview(draft: Ref<FilterDraft>) {
  const data = ref<Overview | null>(null)
  const error = ref<ApiError | null>(null)
  const invalid = ref<string | null>(null)
  const loading = ref(false)
  const lastUpdated = ref<number | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let seq = 0

  async function refresh() {
    // Bump before looking at the draft: an answer still in flight describes the
    // bracket that was just replaced (or just refused), so it must not land.
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
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      const fetched = await apiGet<Overview>(`/api/overview?${query}`)
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
    // An absolute bracket is a fixed question; refreshing it on a timer only
    // moves the answer under the operator's feet.
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

`OverviewView.vue`(:14 的 `useOverview(60)` 与 :15 的 `draft = computed(() => relativeDraft(windowMinutes.value))` 是这个文件唯一的时间来源,一起换掉):

1. 删 `windows` 数组(:28-32)与 header 里的胶囊组(:51-62),换成 `<TimeRangeControl :model-value="draft" @update:model-value="setDraft" />`,刷新按钮留在它旁边;`useOverview(60)` → `useOverview(draft)`,解构去掉 `windowMinutes`/`setWindow`,加上 `invalid`。
2. draft 由路由读写:

```ts
const route = useRoute()
const router = useRouter()
// `route.query` 是 `LocationQuery`,数组成员可能为 null;`parseFilterQuery` 只取
// 第一个字符串,所以两边运行时对得上,只需要一处不加宽度的转换。
type FilterQuery = Parameters<typeof parseFilterQuery>[0]

// URL 是唯一状态来源(§6.5):draft 是派生的,写入只回到路由,所以这里不需要
// `FailuresView.vue:38-84` 那套"序列化后双向比对"的防乒乓守卫——那套是给
// 本地可编辑的 `ref<FilterDraft>` 用的,派生 computed 没有可以偏离 URL 的本地态。
const draft = computed(() => {
  const parsed = parseFilterQuery(route.query as FilterQuery)
  // 一条时间参数都没有时不是"无窗口",而是"服务端默认那一小时";在这里补上,
  // `TimeRangeControl` 的 `aria-pressed` 才有归属,而 URL 保持干净。
  // 不去 `onMounted` 里 `router.replace` 补 URL:那会让首屏发两个请求
  // (一次无参、一次带 windowMinutes),深链测试里"只发一次"就永远测不出来。
  if (parsed.fromMs === null && parsed.toMs === null && parsed.windowMinutes === null) {
    return relativeDraft(DEFAULT_WINDOW_MINUTES)
  }
  return parsed
})

function setDraft(next: FilterDraft) {
  void router.replace({ query: toRouteQuery(next) })
}
```

被 `parseStrictInt` 判废的负 `windowMinutes` 也落进这条默认分支(`parsed` 全 null),与服务端默认同源,和 `failureFilter.spec.ts:154` 的"降级到默认一小时"是同一句话。

3. 区间标签进 header,并且必须有 testid(Step 6 用它断言绝对深链):

```html
<p data-testid="range-label" class="mt-1 text-sm text-muted">
  {{ rangeLabel(draft, Date.now()) }}
</p>
```

替换 :48 那句固定的 "Live DNS health across your fleet."——一个可以选到 7 天前的看板不再"live",留着它就是 §6.4 说的"描述与进程真做的事不一致"。`Queries` 卡片的 `hint="selected window"` 改成 `:hint="rangeLabel(draft, Date.now())"`。

import 一行补全:`DEFAULT_WINDOW_MINUTES`、`parseFilterQuery`、`rangeLabel`、`relativeDraft`(已有)、`toRouteQuery`(已有)、`type FilterDraft`,加上 Task 5 的 `TimeRangeControl`。`vue` 的 `computed` 已在。

4. p95 卡片只改口径文案(D16 的代价必须写在脸上),`label`/`:value`/`format` 三个属性一字不动:

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

`useFailureList(draft, 5)`(:16)与 `onSelect`/`see-all-to`(:24、:106)不用改签名——它们本来就吃 `Ref<FilterDraft>`,现在吃到的是带区间和绝对起停的 draft,这正是本次要的效果:卡片上点开的抽屉与"查看全部"跟着看板选中的区间走。

(`rangeLabel` 的第二个参数在模块里是**故意空着**的时钟形参(:190-191 的注释),模板里 `Date.now()` 只是和 `FailuresView.vue:134` 保持同一种调用形状,不参与渲染结果。别把它换成响应式 ref:那会让一个绝对区间的标签每 15 s 重算一次而什么都不变。)

- [ ] **Step 5: 趋势轴按跨度出标签**

Create `doggy-dns-ui/src/lib/chartLabels.ts`(注意 `tsconfig` 打开了按下标非空检查——spec 里 `events[events.length - 1]!` 就是这么写的,所以下面这条必须带 `!`,否则 `bun run type-check` 红):

```ts
/**
 * An x-axis tick. A one-hour view has always read `HH:mm`; a week of the same
 * labels would be seven identical columns, so anything spanning more than a day
 * gets the date too. The threshold is about the labels, not the bucket width.
 */
export function seriesTick(ts: number, spanMs: number): string {
  const date = new Date(ts)
  const hour = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  if (spanMs <= 24 * 60 * 60 * 1000) return hour
  return `${date.toLocaleDateString([], { month: '2-digit', day: '2-digit' })} ${hour}`
}

/**
 * The span the points cover, in milliseconds. Fewer than two points is `0`,
 * which the one-day threshold above reads as "short", so a single point keeps
 * the hour-only label instead of growing a date it cannot contrast with.
 */
export function seriesSpanMs(points: { ts: number }[]): number {
  if (points.length < 2) return 0
  return points[points.length - 1]!.ts - points[0]!.ts
}
```

`TrendChart.vue` 不要在对象字面量里塞 IIFE:把标签算成一个自己的 computed,再让 `option` 用它。:30-32 那段 `props.series.map(...)` 换成:

```ts
const tickLabels = computed(() => {
  const spanMs = seriesSpanMs(props.series)
  return props.series.map((point) => seriesTick(point.ts, spanMs))
})
```

```ts
  xAxis: {
    type: 'category',
    boundaryGap: false,
    data: tickLabels.value,
    ...
  },
```

Create `doggy-dns-ui/src/lib/__tests__/chartLabels.spec.ts`。**不要**把时刻断言成 `"03:00"` 字面量:jsdom 默认 en-US,`hour: '2-digit', minute: '2-digit'` 给出的是 12 小时制带 `AM/PM` 的串,换 CI 语言就翻。断言写成"由同一个 `toLocale*` 调用算出来"的形式,只钉住"有没有日期段"这件事:

```ts
import { describe, expect, it } from 'vitest'

import { seriesSpanMs, seriesTick } from '@/lib/chartLabels'

const DAY = 24 * 60 * 60 * 1000
const HOUR = 60 * 60 * 1000
// 本地时区构造,避开 `Date.parse` 的时区/夏令时归一化。
const TS = new Date(2026, 0, 1, 3, 0).getTime()
const hourOnly = new Date(TS).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const dated = new Date(TS).toLocaleDateString([], { month: '2-digit', day: '2-digit' })

describe('seriesTick', () => {
  it('keeps the hour-only label inside a day', () => {
    expect(seriesTick(TS, HOUR)).toBe(hourOnly)
    expect(seriesTick(TS, DAY)).toBe(hourOnly)
  })

  it('prefixes the date once the span passes a day', () => {
    expect(seriesTick(TS, DAY + 1)).toBe(`${dated} ${hourOnly}`)
    expect(seriesTick(TS, 7 * DAY)).toBe(`${dated} ${hourOnly}`)
  })

  it('has no span for fewer than two points', () => {
    expect(seriesSpanMs([])).toBe(0)
    expect(seriesSpanMs([{ ts: TS }])).toBe(0)
    expect(seriesSpanMs([{ ts: TS }, { ts: TS + 1000 }])).toBe(1000)
  })
})
```

边界钉子是 `DAY` 与 `DAY + 1`:一个恰好跨天的区间仍读小时,因为分桶最细到 1 分钟,一天 1440 列靠小时标签分得开。

- [ ] **Step 6: 更新 OverviewView 的既有用例**

`src/views/__tests__/OverviewView.spec.ts` 现在按**英文标签文本**找胶囊(:91-96 的 `chipByText`),Task 5 之后按钮文案变成中文且带 testid,所以:

- 删掉 `chipByText`(:91-96)整个 helper —— 留着不引用会被 `bun run lint` 的未用变量规则点掉。
- :178 `chipByText(wrapper, '24h')` → `wrapper.get('[data-testid="quick-24h"]')`;:199、:205、:206 的 `'6h'`/`'1h'` 同理换成 `quick-6h`/`quick-1h`。断言的值(`windowMinutes=1440`、`/failures?windowMinutes=360`、`aria-pressed`)一字不改——点击到路由再回灌的路径和 `FailuresView.spec.ts:146-150` 一样是 `trigger('click')` 后 `await flushPromises()`,现有写法已经够。
- `mountOverview()`(:78-89)加一个入口路径参数:`async function mountOverview(path = '/')`,把 `await router.push('/')` 换成 `await router.push(path)`。现有 6 条用例都不传参,行为不变。
- `stubApi`(:59-69)加请求记录:`const fetches: string[] = []`(模块级,`afterEach` 里清空),stub 内 `fetches.push(String(input))`。没有它,新增的"只发一次请求"断言无从读起。该文件目前没有 `HOUR`,顶部补 `const HOUR = 3_600_000`。

新增三条(两条 D8 方向 + 一条标题文案;`mountOverview` 已带 `path` 参数,`fetches` 已就位):

```ts
const WEEK = 7 * 24 * HOUR
// A fixed clock: the fake timers below move Date.now() with them, so a deep
// link built from the real clock would drift out of the 7-day ceiling.
const NOW = Date.parse('2026-11-04T12:00:00.000Z')

it('sends a relative week as windowMinutes and keeps polling it', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  stubApi(() => json(overview))
  const { wrapper } = await mountOverview('/?windowMinutes=10080')

  const overviewCalls = () => fetches.filter((url) => url.includes('/api/overview'))
  expect(overviewCalls()).toHaveLength(1)
  expect(overviewCalls()[0]).toContain('windowMinutes=10080')

  // D8 only stops the timer for an ABSOLUTE bracket: a relative week has to
  // re-anchor, otherwise "最近 7 天" would freeze at the moment it was opened.
  await vi.advanceTimersByTimeAsync(45_000)
  expect(overviewCalls()).toHaveLength(4)
  vi.useRealTimers()
  wrapper.unmount()
})

it('asks for an absolute week once and never polls it', async () => {
  // The overview used to poll unconditionally; a 7-day bracket refreshed every
  // 15 s is a load test against OpenObserve, not a live view.
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  stubApi(() => json(overview))
  const { wrapper } = await mountOverview(`/?from=${NOW - WEEK}&to=${NOW}`)

  const overviewCalls = () => fetches.filter((url) => url.includes('/api/overview'))
  expect(overviewCalls()).toHaveLength(1)
  expect(overviewCalls()[0]).toContain(`from=${NOW - WEEK}`)
  expect(overviewCalls()[0]).toContain(`to=${NOW}`)

  await vi.advanceTimersByTimeAsync(45_000)
  expect(overviewCalls()).toHaveLength(1)
  vi.useRealTimers()
  wrapper.unmount()
})

it('labels the header with the bracket a deep link describes', async () => {
  const to = Date.now()
  stubApi(() => json(overview))
  const { wrapper } = await mountOverview(`/?from=${to - 3 * HOUR}&to=${to}`)

  // `rangeLabel` renders an absolute pair as `起 ~ 止`; a relative draft never
  // contains the tilde, so the copy itself proves the bracket came from the URL.
  expect(wrapper.get('[data-testid="range-label"]').text()).toContain('~')
  expect(fetches.some((url) => url.includes(`from=${to - 3 * HOUR}`))).toBe(true)
  wrapper.unmount()
})
```

> 第一条用例把 fake timers 装在 `mountOverview` 之前:`router.push` 在 memory history 下只走微任务,不会被假时钟冻住;`await flushPromises()` 已经封在 helper 里。若某条断言因为路由还没落地而红,补一次 `await flushPromises()`,不要改成 `setTimeout` 等待。

Run: `bun x vitest run src/views/__tests__/OverviewView.spec.ts src/composables/__tests__/useOverview.spec.ts src/lib/__tests__/chartLabels.spec.ts`
Expected: PASS。

- [ ] **Step 7: 红/绿自查 + gate + 提交**

两处钉子都要能被推翻,否则用例是空的:

1. 把 `restartTimer` 的 `isAbsolute` 判断去掉 → `asks for the absolute bracket and stops polling it` 变红(45 s 后多出 3 次请求)。改回。
2. 把 `refresh()` 开头的 `const requestSeq = ++seq` 改成 `const requestSeq = seq`(不再递增序列)→ `retires an in-flight answer when the bracket that asked for it is refused` 变红:迟到的回答过了 `requestSeq !== seq` 检查,把已被拒绝的区间重新画上图表。改回。

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

`src/views/__tests__/InstancesView.spec.ts` 现在**不带 router** 挂载(:21、:78 只有 `createPinia()`)。Task 7 之后 `InstancesView.vue` 要 `useRoute()` 读区间,`useRoute()` 在没有 router 的挂载里返回 `undefined`,`route.query` 直接抛 —— 所以这个文件必须先补一个带 memory router 的 `mountView`,照 `OverviewView.spec.ts:78-89` 的形状(用**应用自己的 `routes`**,不要手抄路由表)。**没有 `mountDrawer` 这个东西**,抽屉是本地 `selected` 状态 + 一次 `wrapper.find('button.panel').trigger('click')`(:80),新 helper 把它包起来。

helper 与三条用例(替换该文件现有的两次 `vi.stubGlobal`,共用一份 detail 夹具):

```ts
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { afterEach, describe, expect, it, vi } from 'vitest'

import InstancesView from '@/views/InstancesView.vue'
import { routes } from '@/router'

const HOUR = 3_600_000
const NOW = 1_761_436_800_000

const listed = [
  { id: 'n1', host: 'dns-node-1', version: '0.1.0', online: true, lastSeen: 1, mcpReachable: true },
]

function detailBody(overrides: Record<string, unknown> = {}) {
  return {
    instance: listed[0]!,
    pid: 4242,
    status: 'running',
    uptimeSeconds: 3600,
    listenAddr: '0.0.0.0:15353',
    protocols: ['udp'],
    plugins: [],
    configToml: '',
    redactedKeys: [],
    runtime: { signals: {} },
    // 旧字段 `recentFailures` 到这一步已经不存在:Task 4 把 `InstanceDetail`
    // 换成 `failureCount: Option<u64>`,序列化就是 `failureCount: number | null`。
    failureCount: 0,
    ...overrides,
  }
}

const wrappers: VueWrapper[] = []
let fetches: string[] = []

async function mountView(
  path: string,
  detail: () => unknown = () => detailBody(),
): Promise<{ router: Router; wrapper: VueWrapper }> {
  fetches = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      fetches.push(url)
      const body = url.includes('/api/instances/') ? detail() : listed
      return new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }),
  )
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(InstancesView, { global: { plugins: [router, createPinia()] } })
  wrappers.push(wrapper)
  await flushPromises()
  await wrapper.find('button.panel').trigger('click')
  await flushPromises()
  return { router, wrapper }
}

afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.unstubAllGlobals()
})

describe('InstancesView range', () => {
  it('asks the detail for the bracket the URL carries', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=10080')
    const detail = fetches.find((url) => url.includes('/api/instances/'))!
    expect(detail).toContain('windowMinutes=10080')
    wrapper.unmount()
  })

  it('re-requests the detail when the operator picks another window', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=60')
    const before = fetches.filter((url) => url.includes('/api/instances/')).length

    await wrapper.get('[data-testid="quick-7d"]').trigger('click')
    await flushPromises()

    const list = fetches.filter((url) => url.includes('/api/instances/'))
    expect(list.length).toBe(before + 1)
    expect(list.at(-1)).toContain('windowMinutes=10080')
    wrapper.unmount()
  })

  it('sends no time parameter to the list itself', async () => {
    // `/api/instances` is the one endpoint with no bracket: its rows are
    // identity and reachability. A `?windowMinutes=` here would be a lie the
    // server ignores — the hint line is what says so out loud.
    const { wrapper } = await mountView('/instances?windowMinutes=10080')
    expect(fetches.filter((url) => url === '/api/instances')).toHaveLength(1)
    wrapper.unmount()
  })

  it('renders an em dash instead of a fabricated zero', async () => {
    const { wrapper } = await mountView('/instances?windowMinutes=60', () =>
      detailBody({ failureCount: null }),
    )
    expect(wrapper.get('[data-testid="failure-count"]').text()).toBe('—')
    wrapper.unmount()
  })

  it('renders a counted zero as a zero', async () => {
    // `??`, not `||`: `0` is an answer ("this node was healthy that week"),
    // `null` is the absence of one. A reviewer-proof pin for that difference.
    const { wrapper } = await mountView('/instances?windowMinutes=60')
    expect(wrapper.get('[data-testid="failure-count"]').text()).toBe('0')
    wrapper.unmount()
  })
})
```

(`data-testid="failure-count"` 在**数字那一段** `<p>` 上,不在卡片 `<div>` 上:卡片文本还含标签和跳转链接,`toBe('—')` 才可能成立,e2e 也才能直接 `toHaveText('12')`。)

(`url === '/api/instances'` 要求列表请求**一字不带**参数。`useInstances()` 本来就是这样(:13),这条用例是防止有人"顺手"给它加区间。)

- [ ] **Step 2: 跑测试确认失败**

Run: `bun x vitest run src/views/__tests__/InstancesView.spec.ts`
Expected: FAIL,而且是三种红法都出现:不带 router 的两条旧用例在 `useRoute()` 处抛(证明 router 是真需要,不是重写好玩);`wrapper.get('[data-testid="quick-7d"]')` 找不到控件;`failure-count` 卡片不存在。旧用例里 :58 的 `recentFailures: 0` 与 :21/:78 的挂载写法一起被 `mountView` 取代。

- [ ] **Step 3: 实现**

`src/api/types.ts`:`recentFailures: number` → `failureCount: number | null`(与 Task 4 的 `Option<u64>` + `#[serde(rename_all = "camelCase")]` 一字对应;`null` 是"数不出来",不是 0)。

`useInstances.ts` 的 `useInstanceDetail` 加第二个参数(import `toRequestQuery`、`type FilterDraft`、`type Ref` 已在):

```ts
export function useInstanceDetail(id: Ref<string | null>, draft: Ref<FilterDraft>) {
  const detail = ref<InstanceDetail | null>(null)
  const error = ref<string | null>(null)
  const loading = ref(false)

  async function refresh() {
    if (!id.value) {
      detail.value = null
      return
    }
    loading.value = true
    try {
      // The bracket travels with the request: the failure number is a range
      // answer, everything else in the detail is live MCP data.
      const query = new URLSearchParams(toRequestQuery(draft.value)).toString()
      detail.value = await apiGet<InstanceDetail>(
        `/api/instances/${encodeURIComponent(id.value)}?${query}`,
      )
      error.value = null
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      loading.value = false
    }
  }

  // `immediate` rather than `onMounted`: this composable owns no timer, so it
  // needs no lifecycle hook — and that is also why its spec can call it without
  // a host component, unlike `useOverview`.
  watch([id, draft], () => void refresh(), { immediate: true })
  return { detail, error, loading, refresh }
}
```

`useInstances()`(列表)一字不改:它仍然请求裸的 `/api/instances`,这正是 Step 1 第三条用例钉住的语义。

`InstancesView.vue`:页面级 draft 与 Task 6 Step 4 **完全同款**(派生 + 无参数时补 `relativeDraft(DEFAULT_WINDOW_MINUTES)`,这样抽屉永远带一个显式窗口,`TimeRangeControl` 的 `aria-pressed` 也才有归属);`useInstanceDetail(selectedId, draft)`;`<TimeRangeControl :model-value="draft" @update:model-value="setDraft" />` 挂进 header(:41-50)与 Refresh 并排。本地 `selected` 保持本地——抽屉开合不是区间语义,别把它塞进 URL。

详情指标组(:93-110)加第五张卡,放在 `uptime` 之前(它是这一组里唯一跟着区间走的数字,排第一才说得清"其它都是 live"):

```html
<div class="panel p-3">
  <p class="text-[10px] uppercase tracking-wide text-muted">区间内失败</p>
  <p class="mono mt-1 text-sm" data-testid="failure-count">{{ detail.failureCount ?? '—' }}</p>
  <RouterLink
    class="mono mt-1 inline-block text-[10px] text-accent"
    :to="{
      name: 'failures-list',
      query: { ...toRouteQuery(draft), instance: detail.instance.host },
    }"
    >在失败列表里看这台主机</RouterLink
  >
</div>
```

`?? '—'` 而不是 `dash(...)`:该组件没有 `dash` helper,现有四张卡各自内联 `?? '—'`(:100、:104、:108),第五张跟着同一种写法就是最一致的写法;为一张卡抽 helper 属于评审会点的过度设计。

- [ ] **Step 4: e2e 钉住"详情真的按区间取数"**

`doggy-dns-ui/e2e/console.spec.ts` 追加(demo 模式,fixture 的分组铺到 8 天前;`InstanceCard.vue:30` 把 host 放在按钮文本第一位,所以锚定 `^dns-node-1` 就够区分三台,不需要 `.first()` 猜):

```ts
test('the instance drawer counts failures over the chosen bracket', async ({ page }) => {
  const card = page.getByRole('button', { name: /^dns-node-1\b/ })

  await page.goto('/instances?windowMinutes=10080')
  await card.click()
  const count = page.getByTestId('failure-count')
  // A counted week is a number, never the em dash that means "the telemetry
  // source could not count at all".
  await expect(count).not.toHaveText('—')
  const week = Number(await count.innerText())

  await page.goto('/instances?windowMinutes=60')
  await card.click()
  const hour = Number(await page.getByTestId('failure-count').innerText())

  // The fixture spreads its groups from 10 minutes to 8 days, so a week counts
  // rows an hour cannot see. Asserting the number itself moves is the point:
  // a drawer that only *sends* the bracket and shows a constant would pass a
  // request-only assertion.
  expect(Number.isNaN(week)).toBe(false)
  expect(week).toBeGreaterThan(hour)
})
```

两条既有 e2e 用例不受影响:它们没访问 `/instances`,而 `console.spec.ts:61/66` 的 `range-chip` 属于失败抽屉(`FailureDrawer.vue`),与本任务的实例抽屉是两处 UI —— Task 7 **不许**顺手把 `range-chip` 换成 `TimeRangeControl`,那是第 13 条用例存在的理由。

- [ ] **Step 5: 红/绿自查 + gate + 提交**

把 `useInstanceDetail` 的请求 URL 改回不带 query(删掉 `?${query}`),确认 `asks the detail for the bracket the URL carries` 与 `re-requests the detail when the operator picks another window` 同时变红;改回。再把 `?? '—'` 换成 `|| '—'`,确认 `renders a counted zero as a zero` 变红;改回。
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
- Modify: `README.md`(`:101-109` 的区间段、`:132-137` 的错误段、`:273-281` 的深链段;`### Failure list API`(:73)附近补 `GET /api/overview` 的签名——README 现在**根本没有**这一行,全文 grep 不到 `/api/overview`)
- Modify: `docs/superpowers/plans/2026-11-03-absolute-range-failure-list.md`(在该计划末尾加一行"看板/实例区间由 2026-11-04 计划接管")
- Modify: `doggy-dns-ui/e2e/console.spec.ts`(看板 7 天用例)

- [ ] **Step 1: README 改口径**

三处假话先定位,再逐句替换(不要整段重写,README 的其余部分是刚评审过的):

1. `:106-109` —— "The overview view is the live class and still takes a relative window clamped to 5–1440 minutes; `/api/instances` takes no window, and the instance detail's recent-failure number is a fixed 60 minutes." 三小句现在全是假的。换成:取时间窗的端点(overview、`/api/instances/{id}`、failures 全家)共用同一条 5–10080 与同一套 `400 invalid_request` 文案,`/api/instances` 列表仍是唯一不取区间的端点(它的行是身份与可达性)。
2. `:132-137` —— "the investigation chain rejects an out-of-range bracket instead of clamping it" 的适用范围要从 chain 扩到"every endpoint that takes a bracket";`limit` 那句(:80-83)"夹而不拒"保留,它是唯一剩下的 clamp。
3. `:273-281` —— 深链段只写了 `/failures`。补一句:`/`(看板)与 `/instances`(实例抽屉的数字)同样以 URL 为唯一状态来源,三处共用同一个时间控件;顺带保留"a relative window re-anchors when it is reopened"那半句,它对看板依然成立(相对窗口每次刷新重算),但绝对区间不重算——这正是停止轮询的理由。

新增的两句口径(p95 与 failureCount)写在 Overview 那一段,数字全部来自代码而不是记忆:

- p95 是"区间内最差分桶":桶宽 `bucket_minutes = clamp(ceil(span_minutes / 168), 1, 60)`(`src/domain.rs` 的 `bucket_minutes`),rate 窗口 `max(bucket, 5)` 分钟。默认一小时因此仍是 60s 步长 × 5min rate 窗口,和改版前逐字节相同;一周是 60 分钟一个点。**一句话写明代价:区间越宽,读到的 p95 越高**,所以卡片 hint 是 `worst bucket in range`(D16 的账要写在脸上)。
- 实例详情的 `failureCount`:`SELECT COUNT(*)`,不封顶(失败列表按组聚合、上限 200 组,一周的数会被静默截断);`null` 表示遥测算不出来,前端渲染 `—`。"`null` 不是 0"这句必须留在 README 里 —— 它是 D17 唯一能在文档里被复查的断言。

Run: `grep -n "1440\|clamp" README.md`
Expected(当前 HEAD 的实测基线,4 处):`:80` 是 `limit` 的 clamp(唯一真正保留的 clamp)、`:105` 与 `:136` 是"never clamps / instead of clamping"的**否定句**、`:107` 是本次要替换掉的假话。改完后 `1440` 必须为 **0 处**,`clamp` 只剩上述三类(否定句 + `limit`)。把改前改后两次 grep 的原样输出都贴进报告,不许只写"已清理"。

- [ ] **Step 2: 看板 7 天 e2e(D8 的账要数请求,不是等请求)**

`doggy-dns-ui/e2e/console.spec.ts` 追加两条。**不要**用 `page.waitForResponse(...)`:注册在 `goto` 之后就是 race(第一条请求可能已经走完),注册在之前又只能证明"至少有一次"——而 D8 要的恰恰是"绝对区间**只有**一次"。两条都用 `page.on('response')` 先挂计数器,再 `goto`,再 `expect.poll` 等首个请求落地。

```ts
// D8 splits the two directions of the same rule, so each case gets its own
// 20s window (and its own timeout budget): a relative bracket must keep
// re-anchoring every 15s, an absolute one must stop asking entirely.
function countOverview(page: Page): string[] {
  const urls: string[] = []
  page.on('response', (response) => {
    if (new URL(response.url()).pathname === '/api/overview') urls.push(response.url())
  })
  return urls
}

test('a relative week refreshes the overview', async ({ page }) => {
  test.setTimeout(60_000)
  const urls = countOverview(page)

  await page.goto('/?windowMinutes=10080')
  await expect.poll(() => urls.length).toBeGreaterThan(0)
  expect(urls[0]).toContain('windowMinutes=10080')

  await page.waitForTimeout(20_000)
  // 15s polling means two samples inside 20s. Asserting ">1" rather than "==2"
  // keeps the case about the timer existing, not about wall-clock luck.
  expect(urls.length).toBeGreaterThan(1)
})

test('an absolute bracket is asked for once and never polled', async ({ page }) => {
  test.setTimeout(60_000)
  const to = Date.now()
  const from = to - 60 * 60 * 1000
  const urls = countOverview(page)

  await page.goto(`/?from=${from}&to=${to}`)
  await expect.poll(() => urls.length).toBeGreaterThan(0)
  expect(urls[0]).toContain(`from=${from}`)
  expect(urls[0]).toContain(`to=${to}`)

  await page.waitForTimeout(20_000)
  expect(urls).toHaveLength(1)
})
```

两条都是**现在就会红**的用例,别指望它们只是回归网(红色原因各不同,写报告时按实际输出贴):
- 第一条今天红在 `urls[0]` 是 `...?windowMinutes=60` —— 现存的 `useOverview(60)` 完全不读 URL(`OverviewView.vue:14`),`?windowMinutes=10080` 只是被忽略;Task 6 Step 4 让看板以 URL 为唯一来源后,这一条才转绿。
- 第二条在 Task 6 之前红在 `urls` 一直是 2 条以上:`useOverview.ts:29` 的 `setInterval(refresh, 15_000)` 没有绝对区间分支,Task 6 Step 3 的 `isAbsolute` 才停表。
- 绝对区间的 `from`/`to` 用 `Date.now()` 现算,不要写死 2025 年的毫秒常量:`Range::resolve` 只拒"未来 60s 之后",写死的旧区间虽然合法,但 demo fixture 的数据是按现在铺开 8 天的,卡片全空会让"数字看得见"的其它用例失去意义。
- `import type { Page } from '@playwright/test'`(计数器签名需要),`expect`/`test` 沿用文件第 1 行的导入。

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

- **Spec 覆盖**:D2 → Task 3/4;D8 → Task 6(停表)+ Task 8(两条 e2e 各数 20s 的请求次数)+ Task 2(总量单点);D9 → Task 3/4(`get_overview`/`get_instance_detail` 参数与描述);D15 → Task 1/2;D16 → Task 2(worst bucket)+ Task 6(卡片 hint)+ Task 8(README);D17 → Task 4(不封顶 `COUNT(*)`、`Option`)+ Task 7(可见数字、跳列表、URL 同步);§6.5/6.6 → Task 5/6;§7 全部新钉子分布在 Task 1–8。
- **占位符扫描**:无 TBD;"按现有 helper 形状改写"的三处(`tests/overview_test.rs` 的响应体与 telemetry 构造、`tests/api_test.rs` 的 get helper、e2e 的抽屉定位器)都指明了参照文件与行号,属于对既有形状的显式引用,不是未定义内容。
- **类型一致性**:`Range::bucket_minutes/rate_window_minutes/step_seconds/whole_step_seconds/span_minutes`(Task 1 定义,Task 2/4 消费)、`RangeQuery::range(now_ms)`(Task 3 定义,Task 4 消费)、`failureCount: number | null`(Task 4 后端 `Option<u64>` ↔ Task 7 TS)、`useOverview(draft: Ref<FilterDraft>)`(Task 6 定义,同任务消费)已逐处对齐。
