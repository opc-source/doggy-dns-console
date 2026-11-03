# 绝对时间区间与失败列表页设计

- 日期:2026-11-03
- 状态:设计已逐节确认,待实现
- 范围:后端时间契约(`from`/`to`)+ 按实例过滤 + demo fixture 真过滤 + copilot 工具同步 + 前端失败列表页与抽屉单挂载
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

### 非目标

- **overview 与 instances 不引入绝对区间**:p95 是 `histogram_quantile(0.95, rate(m[{minutes}m]))`,7 天压成一个数没有操作意义;series 是 `rate[5m]` + 60s 步长,7 天 = 10080 点直接灌进 category 轴。两者回答的是"现在怎么样",保持 5–1440 相对窗口。
- 不做 offset 分页(200 组封顶 + 收窄筛选)、不做步长自适应、不动 `MAX_LOGS`。
- 不做实例对比/配置 diff(A3),不做结论反馈与 attempt 历史(A4)。

## 3. 关键决策

| ID | 决策 |
|----|------|
| D1 | 保留 `windowMinutes` 作快捷方式,新增 `from`/`to`(**epoch 毫秒**)。两者都给时 `from/to` 优先;只有 `windowMinutes` 时展开成 `[to - n, to]`,`to` 缺省为 now。 |
| D2 | 两套跨度校验:overview / instances 维持 5–1440 分钟相对窗口;failures / evidence / analysis 允许 5 分钟–10080 分钟的绝对或相对区间。 |
| D3 | 7 天上限用常量,不新增 env——目前没有按部署调它的证据。 |
| D4 | 前端**不再静默 clamp**:`FailureView.vue` 那条"超过 1440 夹回 24h"的规则删掉,非法区间由控件层先拦、后端 400 兜底。后端的拒绝清单只有三条硬规则:起 ≥ 止、跨度 > 7d、`to` 超过 `now + 60s`(浏览器时钟可能略快,不给容差会把正常操作判成非法;超出容差才拒,不承诺"未来一律拒绝")。 |
| D5 | 抽屉只有 `/failures/{traceId}?from&to` 一种打开方式;删 `ui.drawerTraceId` 与 `openFailure`。 |
| D6 | 按实例过滤用 `lower(host_name) = lower(?)` 等值谓词,并额外暴露"未被此过滤覆盖"的行数——`host_name` 缺失时等值过滤会把行静默清空,而 console 判实例身份用的是 hostname-ip / host-pid 启发式。 |
| D7 | 回溯链不查、不渲染 series:列表页没有趋势图,7 天区间不需要步长自适应。 |
| D8 | 区间为绝对时**停止自动刷新**,只留手动;相对窗口保留 15s 轮询。 |
| D9 | copilot 工具(`get_overview`/`list_failures`/`get_failure_detail`)与 HTTP 用同一个 `FailureFilter`,参数加 `from`/`to`/`instance`,并同步这些工具的 **description**(`src/agent/tools.rs:129` 那句"Pass the same windowMinutes/domain/responseCode"就在工具描述里,不在 `src/agent/prompt.rs`)。 |
| D10 | demo fixture 必须真按 filter(时间、instance、domain、responseCode)收窄,否则本期唯一的端到端证据是自证。fixture 现有的五个分组全挤在 `now-30min..now-60s`,必须把时间铺开(含一条落在 7 天之外),否则"1h 与 7d 结果相同"、e2e 断不出差别。 |
| D11 | **证据指纹与请求区间解耦**:`EvidenceFilter` 去掉区间字段,只保留 `domain`/`response_code` 并新增 `instance`。理由:日志与 span 按 `trace_id` 命中,指标窗口是 trace 自身中心 ±15 分钟([`src/evidence/assemble.rs:28`](../../../src/evidence/assemble.rs)),证据集合本质上与用户框多大无关;继续记录区间会让"把窗口从 1h 拖到 7d"变成一次白烧的重跑。区间对证据的真实影响仍体现在 `logs.total/omitted/truncated`、`missing` 和带时间戳的日志 id 里,不会因为不记录区间而漏。 |
| D12 | 不 bump `PROMPT_VERSION`:failure-analyst 只声明 OpenObserve MCP 下钻工具,区间与实例对它是数据不是指令。要改措辞的是 copilot 的业务工具描述(`src/agent/tools.rs`),`SYSTEM_PROMPT` 只列工具名([`src/agent/prompt.rs:13`](../../../src/agent/prompt.rs)),不需要同步语义。 |
| D13 | 区间完全落在保留期外 → 沿用"空结果 + `missing`",不新造 404:console 探测不到 OpenObserve 的保留期。 |
| D14 | D11 的 `EvidenceFilter` 字段变化会让**现存** `data/console.db` 里的分析在下一次打开时全部判 stale(指纹内容变了)。这是一次性的、行为正确,但必须写进 README:同一条 trace 在不同区间下复用同一份已存分析是设计意图,不是缓存 bug。 |

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

overview 与 instances 不在这条链上:它们仍把 `windowMinutes` 交给 `Window::new`(5–1440),再由 `range()` 展开成 `[now-n, now]`。

UI 侧:

```
  /failures?from&to&domain&responseCode&instance&limit     <- 筛选条读写 query,唯一状态源
            | 点击行
            v
  /failures/{traceId}?from&to&analysis=<id>  ->  FailureDrawer (唯一挂载路径)
            | 继续追问
            v
  ui.handoff { traceId, verdict, from, to }  ->  AI 面板(带区间,不再是 windowMinutes)
```

## 5. 文件边界

| 变动 | 文件 |
|---|---|
| 新 `Range { start_ms, end_ms }` + 跨度校验;`FailureFilter` 加 `instance`,窗口字段改 `range` | `src/domain.rs` |
| `EvidenceFilter` 去区间字段、加 `instance`(D11 的落点,`From<&FailureFilter>` 随之改) | `src/evidence/model.rs`、`src/evidence/assemble.rs` |
| `FailureQuery` 加 `from`/`to`/`instance`;`filter_from` 拆成两个具名 helper(回溯链 / 实时看板),不用 bool 参数 | `src/api/failures.rs`(list/detail/evidence/analysis 四条共用) |
| `extra_filters` 加 host 谓词;查询签名 `Window` → `Range`;`range()` 拆职责 | `src/datasource/openobserve/failures.rs`、`telemetry.rs` |
| fixture 按 filter 真收窄 + 分组时间铺开 | `src/datasource/fixture.rs` |
| 工具参数与**工具描述**加区间/实例 | `src/agent/tools.rs` |
| 新路由 `/failures` + `FailuresView.vue` + `FailureFilterBar.vue`;行渲染从 `FailureList.vue` 抽出复用 | `src/router/index.ts`、`doggy-dns-ui/src/views/`、`components/` |
| 抽屉单挂载 + 头部显示区间;看板卡片改"前 5 + 查看全部" | `stores/ui.ts`、`FailureDrawer.vue`、`OverviewView.vue` |
| `useFailureList` 的入参从 `Ref<number>` 改成一个 filter 对象;区间绝对时停轮询 | `composables/useFailureList.ts` |
| `FailureFilter`/`EvidenceFilter` 的 TS 镜像类型与 markers 返回补 `instance`/`from`/`to` | `doggy-dns-ui/src/api/types.ts` |

## 6. 实现要点

**6.1 两套 clamp 不能靠一个布尔开关。** `filter_from` 现在对 list/detail/evidence/analysis 是同一条路径([`src/api/evidence.rs:20`](../../../src/api/evidence.rs)、[`src/api/analysis.rs:161`](../../../src/api/analysis.rs)),改造成两个具名函数(`investigation_filter` / `live_filter`)比加参数清楚,也让"哪个端点属于哪一类"在调用点可读。注意这意味着**同一个 `windowMinutes` 参数在不同端点上限不同**(回溯链 10080、看板 1440):不要为了"统一"把它收回一个常量,这个差别要同时写进 README 和控件提示。

**6.2 按实例过滤必须带"未覆盖"计数。** 过滤后为 0 且窗口内存在 `host_name` 为空的行时,列表页要说"有 N 条遥测没有主机标签,未被此过滤覆盖"并给一个查看全部的动作,而不是只报数。代价是一次轻量 count。

**6.3 `MAX_LOGS`、步长与 6 次工具预算都不变,而且有两条独立理由。** 指标证据的窗口是 trace 自身中心 ±15 分钟([`src/evidence/assemble.rs:28`](../../../src/evidence/assemble.rs)、[:114-115](../../../src/evidence/assemble.rs)),压根不吃请求区间;`failure_metrics` 内部 `step = 整个跨度` 只取一个点([`src/datasource/openobserve/telemetry.rs:162`](../../../src/datasource/openobserve/telemetry.rs#L162))。日志与 span 按 `trace_id` 命中,同一 trace 在 60 分钟和 7 天区间里的行数基本相同。所以本期不动截断规则、不动预算——如果一次 7 天区间的调查真的撞上 `logs_truncated`,那是证据层的独立议题,记进 §8。

**6.4 copilot 与 analyst 是两个模型入口。** `build_agent` 注册的是业务工具([`src/agent/mod.rs:22-35`](../../../src/agent/mod.rs)),analyst 只声明 OpenObserve MCP 下钻工具([`src/analysis/prompt.rs:22-23`](../../../src/analysis/prompt.rs))。区间/实例参数只进前者,`PROMPT_VERSION` 保持 `failure-analyst/v2`。要改的措辞在**工具描述**里——`get_failure_detail` 现在写着"Pass the same windowMinutes/domain/responseCode used by list_failures"([`src/agent/tools.rs:129`](../../../src/agent/tools.rs#L129)),不补 `from/to/instance`,模型就以为自己只能用相对窗口;这是 #7/#10 那类"提示声称进程不做的事"的同形问题。copilot 的 `SYSTEM_PROMPT` 只列工具名,不需要跟着改。

**6.5 前端 URL 是唯一状态源。** 筛选条读写 query,不引入新的 store 字段;`datetime-local` 按浏览器本地时区换算成 ms。抽屉的 handoff 上下文(`ui.handoff`)随之从 `windowMinutes` 换成区间,否则面板里"回到调查页"的链接会丢掉区间([`FailureDrawer.vue:226`](../../../doggy-dns-ui/src/components/FailureDrawer.vue)、[`AiPanel.vue:55`](../../../doggy-dns-ui/src/components/AiPanel.vue))。

## 7. 验证

- `cargo test`:新增 `Range` 校验表驱动用例(跨度 < 5min、= 7d、> 7d、起≥止、`to` 在 `now+60s` 之外被拒 / 之内被接受)、host 等值谓词的 SQL 文本、fixture 按 filter 收窄(含 `host_name` 缺失行)、以及 **D11 的核心钉**:同一条 trace 在 1h 与 7d 区间下 assemble 出的指纹**相同**(区间不再进 filter,证据相同就不该重跑)。
- `cargo test`:copilot 工具描述含 `from`/`to`/`instance` 的断言(照 #8 的做法,把描述文本钉住,防止它再次承诺进程不做的事)。
- `bun run test:unit`:URL ↔ 筛选条双向、7d 区间不被前端夹回 24h、抽屉无 store 路径、绝对区间不轮询、marker 在 200 组下的行为、未覆盖计数文案。
- `bun run test:e2e`(demo 模式):`按 dns-node-2 过滤 + 7d 区间 → 只剩它的分组`,以及"1h 区间看不到 3 天前那条分组"(证明 fixture 真在过滤,而不是自证)。
- `bun run type-check` / `lint`;README 深链段补 `from/to`,写明"回溯链 7d / 看板与实例仍 ≤24h"和"同一 trace 换区间复用已存分析是设计意图"。
- 先看红的回归钉:D11 的指纹不变、D4 的前端 clamp 删除、D5 的 store 挂载删除、6.2 的未覆盖计数、6.4 的工具描述。

## 8. 已知限制

- 没有 offset,200 组封顶;命中上限时显示"仅显示前 200 组,请收窄时间或筛选",不假装是全集。
- p95 与趋势线只在 ≤24h 相对窗口有意义;回溯链看不到趋势。
- 保留期是外部实例的属性,console 探测不到:更早的区间表现为空结果,不是错误。
- `host_name` 缺失时按实例过滤仍会不完整——本期只暴露,不解决(要解决得先统一实例身份口径)。
- 7 天区间里的证据仍受 `MAX_LOGS=200` 截断;超长命中的 trace 会带 `logs_truncated`。
- `bump` copilot `SYSTEM_PROMPT` 不影响已有分析缓存(缓存键是 `model + prompt_version`,analyst 未变),但旧会话回放里的工具调用参数可能与新工具签名不一致——回放只读,不做迁移。
