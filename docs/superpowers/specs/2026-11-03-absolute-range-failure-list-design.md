# 绝对时间区间与失败列表页设计

- 日期:2026-11-03
- 状态:设计已逐节确认,待实现
- 范围:后端时间契约(`from`/`to`)+ 按实例过滤 + demo fixture 真过滤 + copilot 工具同步 + 前端失败列表页与抽屉单挂载 + 看板与实例详情的绝对区间(见 D15–D17)
- 修订:2026-11-04 二次评审。原定"overview/instances 保持 5–1440 相对窗口"(旧 D2 的前半、旧 D7),经复核**作废**:操作者要看的是"这七天的趋势和 p95",不是只能看今天。本节以 D2(改写)、D15、D16、D17 为准;凡与早期段落冲突,以决策表为准。
- 前置:PR #5(证据优先的失败调查)、#7/#10(把真实的预算/时限告诉模型、600s 默认)、#9(AI 面板有序流与三态)

## 1. 背景

console 里唯一把时间钉成"相对"的地方是 HTTP 边界的 `Window { minutes }`。往下早就是一切皆区间:`range()` 返回 `(now_ms - minutes*60_000, now_ms)`,`_timestamp` 比微秒、search body 给微秒、PromQL 给秒,`failure_metrics(host, start_ms, end_ms)` 更是天生吃显式区间。后果与缺口:

1. **查不了历史案**:`Window::new` 把跨度 clamp 到 5–1440 分钟,昨天凌晨那次失败,今天打不开——抽屉的参数里根本没有承载绝对时刻的位置。
2. **列表能力后端已有、前端没用**:`/api/failures` 支持 `domain`、`responseCode`、`limit` 到 200,前端只传 `windowMinutes&limit=5` 再 `.slice(0, 5)`,`FailureGroup` 自带的 qtype/authority/error/duration/first_seen/last_seen/instance 一个都没显示,也没有"失败"路由。
3. **同一个抽屉有两种打开语义**:看板走 `ui.drawerTraceId`(无 URL),`/failures/:traceId` 走 URL。区间天生只能住在 URL 里,store 那条路径没地方放。

## 2. 目标与非目标

### 目标

1. 回溯链(`/api/failures`、`/{trace}/evidence`、`/analysis`)支持绝对区间,跨度上限放宽到 **7 天**。
2. 新增失败列表页:时间输入 + `域名包含` / `responseCode` / **按实例** 筛选 + 补齐分组字段。
3. 抽屉只有一条打开路径(URL),`ui.drawerTraceId` 退役。
4. 同一份 filter 契约同时覆盖 HTTP、copilot 工具与 demo fixture,不留"后端支持但别的入口不支持"的裂口。
5. 看板(`/api/overview`)与实例详情(`/api/instances/{id}`)也能框 7 天内的任意绝对区间,与回溯链共用同一套拒绝规则和同一个时间控件。

### 非目标

- 实例**列表**(`/api/instances`)不引入区间:它的每一行是"这台节点是谁、还在不在",由固定的发现回看(`DISCOVERY_WINDOW_MINUTES`)与在线判定(`ONLINE_WINDOW_MS`)决定,不是操作者选出来的区间。
- 不做 offset 分页(200 组封顶 + 收窄筛选)、不动 `MAX_LOGS`。
- 不做实例对比/配置 diff(A3),不做结论反馈与 attempt 历史(A4)。
- 趋势图不做"多指标叠加/缩放拖拽"——本期只把 7 天塞进现有的 category 轴。

## 3. 关键决策

| ID | 决策 |
|----|------|
| D1 | 保留 `windowMinutes` 作快捷方式,新增 `from`/`to`(**epoch 毫秒**)。两者都给时 `from/to` 优先;只有 `windowMinutes` 时展开成 `[to - n, to]`,`to` 缺省为 now。 |
| D2 | **一套**跨度规则:`/api/overview`、`/api/instances/{id}`、`/api/failures`(list/detail/evidence/analysis)全部走 `Range::resolve` + `Range::checked`,即 5 分钟–10080 分钟、起 ≥ 止拒绝、`to` 超 `now + 60s` 拒绝,一律 **400 不 clamp**。`Window` 只剩内部用途(实例发现 `DISCOVERY_WINDOW_MINUTES`、`seen_instances`),不再是 HTTP 契约的一部分。 |
| D3 | 7 天上限用常量,不新增 env——目前没有按部署调它的证据。 |
| D4 | 前端**不再静默 clamp**:`FailureView.vue` 那条"超过 1440 夹回 24h"的规则删掉,非法区间由控件层先拦、后端 400 兜底。后端的拒绝清单只有三条硬规则:起 ≥ 止、跨度 > 7d、`to` 超过 `now + 60s`(浏览器时钟可能略快,不给容差会把正常操作判成非法;超出容差才拒,不承诺"未来一律拒绝")。 |
| D5 | 抽屉只有 `/failures/{traceId}?from&to` 一种打开方式;删 `ui.drawerTraceId` 与 `openFailure`。 |
| D6 | 按实例过滤用 `lower(host_name) = lower(?)` 等值谓词,并额外暴露"未被此过滤覆盖"的行数——`host_name` 缺失时等值过滤会把行静默清空,而 console 判实例身份用的是 hostname-ip / host-pid 启发式。 |
| D7 | 回溯链不查、不渲染 series:列表页没有趋势图。(旧文本"7 天区间不需要步长自适应"随看板一起放宽作废:趋势图仍在看板,但步长自适应,见 D15。) |
| D8 | 区间为绝对时**停止自动刷新**,只留手动;相对窗口保留 15s 轮询。看板的 KPI/趋势与失败列表都遵守这条:一个 7 天区间每 15s 重放 5 次 range query 是把 OpenObserve 当压力测试,不是刷新。 |
| D9 | copilot 工具(`get_overview`/`list_failures`/`get_failure_detail`)与 HTTP 用同一个 `FailureFilter`,参数加 `from`/`to`/`instance`,并同步这些工具的 **description**(`src/agent/tools.rs:129` 那句"Pass the same windowMinutes/domain/responseCode"就在工具描述里,不在 `src/agent/prompt.rs`)。 |
| D10 | demo fixture 必须真按 filter(时间、instance、domain、responseCode)收窄,否则本期唯一的端到端证据是自证。fixture 现有的五个分组全挤在 `now-30min..now-60s`,必须把时间铺开(含一条落在 7 天之外),否则"1h 与 7d 结果相同"、e2e 断不出差别。 |
| D11 | **证据指纹与请求区间解耦**:`EvidenceFilter` 去掉区间字段,只保留 `domain`/`response_code` 并新增 `instance`。理由:日志与 span 按 `trace_id` 命中,指标窗口是 trace 自身中心 ±15 分钟([`src/evidence/assemble.rs:28`](../../../src/evidence/assemble.rs)),证据集合本质上与用户框多大无关;继续记录区间会让"把窗口从 1h 拖到 7d"变成一次白烧的重跑。区间对证据的真实影响仍体现在 `logs.total/omitted/truncated`、`missing` 和带时间戳的日志 id 里,不会因为不记录区间而漏。 |
| D12 | 不 bump `PROMPT_VERSION`:failure-analyst 只声明 OpenObserve MCP 下钻工具,区间与实例对它是数据不是指令。要改措辞的是 copilot 的业务工具描述(`src/agent/tools.rs`),`SYSTEM_PROMPT` 只列工具名([`src/agent/prompt.rs:13`](../../../src/agent/prompt.rs)),不需要同步语义。 |
| D13 | 区间完全落在保留期外 → 沿用"空结果 + `missing`",不新造 404:console 探测不到 OpenObserve 的保留期。 |
| D14 | D11 的 `EvidenceFilter` 字段变化会让**现存** `data/console.db` 里的分析在下一次打开时全部判 stale(指纹内容变了)。这是一次性的、行为正确,但必须写进 README:同一条 trace 在不同区间下复用同一份已存分析是设计意图,不是缓存 bug。 |
| D15 | **步长自适应只服务看板的趋势线**:`bucket_minutes = clamp(ceil(span_minutes / 168), 1, 60)`,range query 的 `step = bucket * 60s`,rate 窗口 = `max(bucket, 5)` 分钟。168 = 7 天按小时分桶,也是 category 轴还看得过去的点数;默认 1h 区间算出 bucket=1,于是沿用今天"`rate[5m]` 每 60s 一点"的形状,**现有视图的曲线不变**。总量 KPI 仍是一个覆盖整个区间的 `increase` 采样(step 用一个跨度,只取最后一点),不随分桶变。 |
| D16 | **p95 改成"分桶取最差"**:`histogram_quantile(0.95, sum by (le) (rate(m[{bucket}m])))` 按 D15 的 step 跑成一条序列,取非 NaN 点的最大值。理由:7 天压成一个分位数没有操作意义(原非目标里那句反对是对的,只是解法不是"不让选 7 天");而"区间里最差的一段"是可以行动的。代价必须承认:默认 1h 的 P95 卡片从"整段 p95"变成"最差 5 分钟 p95",数值通常会**变高**——README 与卡片 hint 都要写清口径,不写就是骗人。 |
| D17 | **实例详情取区间、列表不取**,并且那个数字必须可见:`InstanceDetail.recent_failures` 改名 `failure_count`(JSON `failureCount`),来源从"取 50 组再相加"换成一条 `SELECT COUNT(*)`(与列表同一个 `FAILURE_PREDICATE` 加 host 等值谓词),否则 7 天区间会被 group limit 静默少算——这是把口径放大后必须补的洞。计不了数的来源留 `None`(抽屉显示 `—`,不是 0)。抽屉里补一张"区间内失败"卡片 + 一个跳到 `/failures?instance&from&to` 的入口:未覆盖(D6)的提示仍只在列表页出现,不在详情里再查一遍同一份证据。时间控件写进 `/instances` 的 URL query,`get_instance_detail` 工具同样收 `from`/`to`(D9)。 |

## 4. 数据流

```
  GET /api/failures            (list / detail / evidence / analysis 共用同一 FailureQuery)
  POST /api/chat -> copilot tools          |
            |                              |
            +-------------+----------------+
                          v
                 FailureFilter { range, domain, response_code, instance, limit }
                          |
        +---------------+----------------+
        |                                |
        v                                v
  OpenObserveTelemetry            FixtureTelemetry (demo)
  SQL  _timestamp >= start*1000    按 filter 真收窄
  PromQL start/1000 (秒)           (时间/instance/domain/code)
        |
        v
  EvidenceBundle { filter(无区间), metrics(trace 中心±15m) } -> fingerprint -> analysis 缓存键
```

看板与实例详情也在这条链上(D2 改写后):`/api/overview`、`/api/instances/{id}` 把 `from`/`to`/`windowMinutes` 同样交给 `Range::resolve`,只有跨度上限一致的绝对区间,不再有 `Window::new` 那条 5–1440 的 clamp。区别只在下游——看板拿区间去跑 5 个 range query(总量按整段一个 `increase` 采样,趋势与 p95 按 D15 的分桶),实例详情拿区间去跑两条 `COUNT(*)`(该主机 / 无主机标签)。`/api/instances` 列表不在这条链上:发现窗口与在线判定是内部常量。

UI 侧:

```
  /failures?from&to&domain&responseCode&instance&limit     <- 筛选条读写 query,唯一状态源
            | 点击行
            v
  /failures/{traceId}?from&to&analysis=<id>  ->  FailureDrawer (唯一挂载路径)
            | 继续追问
            v
  ui.handoff { traceId, verdict, from, to }  ->  AI 面板(带区间,不再是 windowMinutes)

  /?from&to&windowMinutes                    <- 看板时间控件:同一套解析/校验,绝对区间停轮询(D8)
  /instances?from&to&windowMinutes           <- 实例抽屉的时间控件写页面 query,详情按它取数
```

## 5. 文件边界

| 变动 | 文件 |
|---|---|
| 新 `Range { start_ms, end_ms }` + 跨度校验;`FailureFilter` 加 `instance`,窗口字段改 `range` | `src/domain.rs` |
| `EvidenceFilter` 去区间字段、加 `instance`(D11 的落点,`From<&FailureFilter>` 随之改) | `src/evidence/model.rs`、`src/evidence/assemble.rs` |
| `FailureQuery` 加 `from`/`to`/`instance`;`filter_from` 拆出具名 helper(回溯链),不用 bool 参数 | `src/api/failures.rs`(list/detail/evidence/analysis 四条共用) |
| `extra_filters` 加 host 谓词;查询签名 `Window` → `Range`;`range()` 拆职责 | `src/datasource/openobserve/failures.rs`、`telemetry.rs` |
| fixture 按 filter 真收窄 + 分组时间铺开 | `src/datasource/fixture.rs` |
| 工具参数与**工具描述**加区间/实例 | `src/agent/tools.rs` |
| 新路由 `/failures` + `FailuresView.vue` + `FailureFilterBar.vue`;行渲染从 `FailureList.vue` 抽出复用 | `src/router/index.ts`、`doggy-dns-ui/src/views/`、`components/` |
| 抽屉单挂载 + 头部显示区间;看板卡片改"前 5 + 查看全部" | `stores/ui.ts`、`FailureDrawer.vue`、`OverviewView.vue` |
| `useFailureList` 的入参从 `Ref<number>` 改成一个 filter 对象;区间绝对时停轮询 | `composables/useFailureList.ts` |
| `FailureFilter`/`EvidenceFilter` 的 TS 镜像类型与 markers 返回补 `instance`/`from`/`to` | `doggy-dns-ui/src/api/types.ts` |
| **D2 改写**:`WindowQuery` → `RangeQuery`(`from`/`to`/`windowMinutes`),`overview(Range)`;`instance_detail` 收区间,`recent_failures` → `failure_count` | `src/api/overview.rs`、`src/api/instances.rs`、`src/service.rs`、`src/domain.rs`、`src/datasource/mod.rs` |
| **D15/D16**:`Range → (bucket, step)` 的纯函数;series/p95 的 rate 窗口与 step 由它给,总量仍整段一个采样 | `src/datasource/openobserve/overview.rs`、`telemetry.rs` |
| **D17**:主机 `COUNT(*)` 查询(与 `unlabeled_failures_query` 同一谓词、含 host 等值) | `src/datasource/openobserve/failures.rs`、`telemetry.rs`、`src/datasource/fixture.rs` |
| **D9**:`get_overview` 加 `from`/`to` 并同步描述 | `src/agent/tools.rs` |
| **D15/D17 前端**:把时间控件从 `FailureFilterBar` 抽成共用组件,看板与实例抽屉各挂一个,读写页面 query;绝对区间停轮询 | `doggy-dns-ui/src/components/`、`views/OverviewView.vue`、`views/InstancesView.vue`、`composables/useOverview.ts`、`composables/useInstances.ts` |

## 6. 实现要点

**6.1 区间解析只有一个类,但仍是具名 helper。** `filter_from` 现在对 list/detail/evidence/analysis 是同一条路径([`src/api/evidence.rs:20`](../../../src/api/evidence.rs)、[`src/api/analysis.rs:161`](../../../src/api/analysis.rs)),抽成 `investigation_filter`(回溯链:`FailureFilter`)比加 bool 参数清楚。D2 改写后 overview / instance detail 不再需要第二个 clamp 类:两者只要 `Range::resolve` 出来的区间,不需要 domain/responseCode/limit,所以**不抽 filter,只共用参数结构**——`src/api/mod.rs` 加 `RangeQuery { from, to, windowMinutes }`(带一个 `range(now_ms) -> Result<Range, DataError>`),两个端点各自 `Query<RangeQuery>`;"三个查询参数的语法"只有一份,而调用点仍然一行可读。`ApiError` 映射与 400 文案沿用同一套 `DataError::Invalid`。**统一之后的口径要同时写进 README 和控件提示**:所有端点都是 5–10080 分钟、都拒绝而不 clamp;`/api/instances` 列表是唯一不取区间的端点(它压根没有区间语义),提示里必须点名它,否则操作者会以为实例卡片也跟随时间控件。

**6.2 按实例过滤必须带"未覆盖"计数。** 过滤后为 0 且窗口内存在 `host_name` 为空的行时,列表页要说"有 N 条遥测没有主机标签,未被此过滤覆盖"并给一个查看全部的动作,而不是只报数。代价是一次轻量 count。

**6.3 `MAX_LOGS`、证据侧步长与 6 次工具预算都不变,而且有两条独立理由。** 指标证据的窗口是 trace 自身中心 ±15 分钟([`src/evidence/assemble.rs:28`](../../../src/evidence/assemble.rs)、[:114-115](../../../src/evidence/assemble.rs)),压根不吃请求区间;`failure_metrics` 内部 `step = 整个跨度` 只取一个点([`src/datasource/openobserve/telemetry.rs:162`](../../../src/datasource/openobserve/telemetry.rs#L162))。日志与 span 按 `trace_id` 命中,同一 trace 在 60 分钟和 7 天区间里的行数基本相同。所以本期不动截断规则、不动预算——如果一次 7 天区间的调查真的撞上 `logs_truncated`,那是证据层的独立议题,记进 §8。本期改的是看板趋势/p95 查询的步长(D15),它压根不在证据链上,两者别混。

**6.4 copilot 与 analyst 是两个模型入口。** `build_agent` 注册的是业务工具([`src/agent/mod.rs:22-35`](../../../src/agent/mod.rs)),analyst 只声明 OpenObserve MCP 下钻工具([`src/analysis/prompt.rs:22-23`](../../../src/analysis/prompt.rs))。区间/实例参数只进前者,`PROMPT_VERSION` 保持 `failure-analyst/v2`。要改的措辞在**工具描述**里——`get_failure_detail` 现在写着"Pass the same windowMinutes/domain/responseCode used by list_failures"([`src/agent/tools.rs:129`](../../../src/agent/tools.rs#L129)),不补 `from/to/instance`,模型就以为自己只能用相对窗口;`get_overview` 同样加 `from`/`to`(与 `/api/overview` 同一套 `Range::resolve` 拒绝规则),否则 copilot 看到的"窗口"和界面看到的不是同一段时间——这是 #7/#10 那类"提示声称进程不做的事"的同形问题。copilot 的 `SYSTEM_PROMPT` 只列工具名,不需要跟着改。

**6.5 前端 URL 是唯一状态源。** 筛选条读写 query,不引入新的 store 字段;`datetime-local` 按浏览器本地时区换算成 ms。抽屉的 handoff 上下文(`ui.handoff`)随之从 `windowMinutes` 换成区间,否则面板里"回到调查页"的链接会丢掉区间([`FailureDrawer.vue:226`](../../../doggy-dns-ui/src/components/FailureDrawer.vue)、[`AiPanel.vue:55`](../../../doggy-dns-ui/src/components/AiPanel.vue))。看板与实例抽屉用同一个 `TimeRangeControl`:时间控件的形态(快捷键 + 起/止 + 回到近 1 小时 + 上限提示)在三个页面里是同一段 DOM,分叉只会让"哪个页面支持 7 天"变成要靠试才知道的事;看板的 1h/6h/24h 胶囊保留为快捷预设,并把 7d 加进去。

**6.6 分桶是纯函数,不是查询里的临时算术。** `bucket_minutes(span) = clamp(ceil(span / 168), 1, 60)` 放在 `src/domain.rs`(和 `Range` 同处),因为它同时决定 step、rate 窗口和 p95 的口径,三处必须算出同一个数;把它写成 `Range` 上的方法(`range.bucket_minutes()`)比在每个查询里重算强。边界值要钉:5 分钟 → 1(仍 60s 步长)、1440 → 9、10080 → 60。rate 窗口取 `max(bucket, 5)` 分钟,是为了让默认 1h 视图**逐点等于今天的结果**——曲线形状变了却没人签字,是本类改动最常见的隐性回归。p95 序列里 `NaN`/缺点必须跳过(`histogram_quantile` 在无样本的桶上返回 NaN),否则"最差桶"会变成一个 NaN 或第一个有值的桶。

## 7. 验证

- `cargo test`:新增 `Range` 校验表驱动用例(跨度 < 5min、= 7d、> 7d、起≥止、`to` 在 `now+60s` 之外被拒 / 之内被接受)、host 等值谓词的 SQL 文本、fixture 按 filter 收窄(含 `host_name` 缺失行)、以及 **D11 的核心钉**:同一条 trace 在 1h 与 7d 区间下 assemble 出的指纹**相同**(区间不再进 filter,证据相同就不该重跑)。
- `cargo test`:copilot 工具描述含 `from`/`to`/`instance` 的断言(照 #8 的做法,把描述文本钉住,防止它再次承诺进程不做的事)。
- `bun run test:unit`:URL ↔ 筛选条双向、7d 区间不被前端夹回 24h、抽屉无 store 路径、绝对区间不轮询、marker 在 200 组下的行为、未覆盖计数文案。
- `bun run test:e2e`(demo 模式):`按 dns-node-2 过滤 + 7d 区间 → 只剩它的分组`,以及"1h 区间看不到 3 天前那条分组"(证明 fixture 真在过滤,而不是自证)。
- `bun run type-check` / `lint`;README 深链段补 `from/to`,写明"**所有**取区间的端点统一 5 分钟–7 天且不 clamp、看板 p95 是区间内最差分桶、趋势按分桶步长采样、`/api/instances` 列表不取区间、实例详情的失败数是不封顶的 `COUNT(*)`"和"同一 trace 换区间复用已存分析是设计意图"。
- 先看红的回归钉:D11 的指纹不变、D4 的前端 clamp 删除、D5 的 store 挂载删除、6.2 的未覆盖计数、6.4 的工具描述。

**二次评审新增(D2/D15/D16/D17)**

- `cargo test`:`bucket_minutes` 的边界表(5→1、60→1、1440→9、10080→60、跨度恰好整除/不整除),以及 `step = bucket*60` 与 `rate 窗口 = max(bucket,5)` 的 PromQL 文本钉。**要钉的两条相反承诺**:series 在默认 1h 下的查询文本与 step 与改动前**逐字节相同**(曲线不许悄悄变),而 p95 **必然变**——D16 改了口径,要钉的是"取的是有限点的最大值、NaN 被跳过",不是它的不变。总量两条只检查 `increase(m[{span}m])` 的窗口仍是整段,step 放大到一个跨度不构成第 10081 个点。
- `cargo test`:`/api/overview?from&to` 走同一套 400(起≥止、>7d、`to` 超容差),而**不再出现 clamp**;`/api/instances/{id}?from&to` 同理;`?windowMinutes=20000` 在 `/failures` 和 `/overview` 上返回同一个拒绝文案。
- `cargo test`:实例失败数改用 `COUNT(*)` 后,SQL 文本含 `FAILURE_PREDICATE` + host 等值且**没有 group/limit**;fixture 的 `failure_count` 随区间变(3 天前的分组在 1h 区间不计入)。
- `cargo test`:p95 序列含 NaN 点时取的是最大有限值,不是 NaN、也不是首点。
- `bun run test:unit`:看板时间控件读写 URL(`/?windowMinutes=10080`、`/?from&to`)、绝对区间停 15s 轮询且手动刷新仍在、实例抽屉换区间重新请求 `/api/instances/{id}`、`TimeRangeControl` 三处挂载共用 `failureFilter.ts` 而不复制校验。
- `bun run test:e2e`(demo 模式):`/instances` 选 7d → 详情卡片显示跨到 3 天前的失败数,选 1h → 数字变小(证明详情真的按区间取数)。

## 8. 已知限制

- 没有 offset,200 组封顶;命中上限时显示"仅显示前 200 组,请收窄时间或筛选",不假装是全集。
- 7 天区间的趋势线是**每小时一个点、每点是一条 60 分钟 rate 窗口的平均**(D15),看的是形状不是尖峰;想抓单次抖动仍要收窄区间或走回溯链。回溯链看不到趋势。
- p95 是"区间内最差分桶"(D16):默认 1h 视图的 P95 数值会比改动前**偏高**,因为它现在是若干 5 分钟窗口里的最大值,而不是整段一个分位数。口径写进卡片 hint 与 README;跨区间的 P95 数字不可直接比较(桶宽随跨度变)。
- 一次 7 天看板刷新仍是 5 个 range query(p95/series 各 168 点),依赖 OpenObserve 侧的保留期与查询预算;console 只保证不再按 60s 步长把整段重放(D8 绝对区间停轮询 + 总量单点)。
- 保留期是外部实例的属性,console 探测不到:更早的区间表现为空结果,不是错误。
- `host_name` 缺失时按实例过滤仍会不完整——本期只暴露,不解决(要解决得先统一实例身份口径)。
- 7 天区间里的证据仍受 `MAX_LOGS=200` 截断;超长命中的 trace 会带 `logs_truncated`。
- `bump` copilot `SYSTEM_PROMPT` 不影响已有分析缓存(缓存键是 `model + prompt_version`,analyst 未变),但旧会话回放里的工具调用参数可能与新工具签名不一致——回放只读,不做迁移。
