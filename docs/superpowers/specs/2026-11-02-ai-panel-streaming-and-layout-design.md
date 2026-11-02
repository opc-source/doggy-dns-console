# AI 面板流式渲染与布局改造设计

- 日期：2026-11-02
- 状态：已实现，单测/类型/浏览器实测通过
- 范围：前端 `doggy-dns-ui`（渲染模型、Markdown、面板三态、失败→追问交接）；后端契约不变
- 前置：PR #5「证据优先的失败调查」、PR #7「把真实的预算/时限/工具告诉模型」

## 1. 背景

面板改造前的实现有四处互相独立的问题，共同的根因是**把一次回答当成一段字符串**：

1. **顺序丢失**：`chat` store 的消息只有 `content` 与 `toolCalls` 两个平行字段，思考文本更是被流式层丢弃。一次回答里「先说结论 → 调工具 → 再补充」的原始顺序无法还原；回放更狠，`mapSessionEvents` 只拼接 text part，工具调用在历史里直接消失。
2. **Markdown 不渲染**：模型输出的是 Markdown（表格、代码块、链接），界面按纯文本打印；同时会话回放里出现过整段 JSON，可读性很差。直接 `v-html` 又等于把 XSS 面打开。
3. **面板布局只有一种**：右侧固定栏。长回答（一次调查动辄上千字加多张工具卡片）在 420px 栏里既读不完也回不去原来的阅读位置；想临时看全屏只能整个隐藏。
4. **失败→追问断链**：调查页（`FailureDrawer`）已经拿到证据包与结论，点「追问」只能人工把 traceId 复制进输入框，面板不知道自己在讨论哪个 trace。

## 2. 目标与非目标

### 目标

1. 一个**有序段（segment）模型**作为唯一渲染真相：thinking / text / tool 按到达顺序交错呈现，流式与回放共用。
2. Markdown 与 JSON 走**白名单净化**后渲染，链接新窗口 + `noopener noreferrer`。
3. 面板三态（docked / hidden / fullscreen）切换**不丢用户输入**，全屏是临时阅读态、不落盘。
4. 失败详情到面板的**交接上下文**（traceId、verdict、窗口）显式呈现，可一键回到调查页。

### 非目标

- 不改 AG-UI 事件契约、不改后端 `/api/chat` 与证据接口。
- 不做多标签页状态同步、不做草稿跨浏览器重启恢复。
- 不做虚拟列表/长会话性能优化（本地 14 个会话实测 9–74 个事件/会话，尚未出现瓶颈）。

## 3. 关键决策

| ID | 决策 |
|----|------|
| D1 | 消息 = `role` + `Segment[]`；段类型 `thinking` / `text` / `tool`，工具段自带 `args`/`result`/`status`。 |
| D2 | 归约是**纯函数** `applyAgUiEvent(segments, event)`：按 `messageId` 合并文本/思考增量，按 `toolCallId` 合并工具增量；不在 reducer 里碰 Vue 响应式。 |
| D3 | 容忍缺 start 事件：`TOOL_CALL_ARGS` / `TOOL_CALL_RESULT` 可以首次出现即建段（跨 run 恢复的挂起调用、只发 result 的生产者）。 |
| D4 | 一次 run 结束就收尾未答的工具卡：reducer 在 `RUN_ERROR` 收尾，`chat.send()` 在流返回后无条件 `settleRunningTools(segments, 'error')`——正常结束、出错、流提前断开、被下一个问题抢占都走这条，避免卡片永久停在 running 脉冲态。 |
| D5 | 回放与流式**共用同一套段类型**：`messagesFromSessionEvents` 把持久化的 `SessionEvent.parts` 映射成段，顺序规则与在线一致（同一事件内多个 text part 合并到第一个的位置）。 |
| D6 | Markdown 渲染集中在 `src/lib/markdown.ts`，DOMPurify 钩子在**模块加载时注册一次**；`MarkdownBlock.vue` 是应用内唯一的 `v-html`。 |
| D7 | 整段是 JSON 时不渲染 Markdown，改为 `JSON.stringify(…, null, 2)` 的 `<pre>`；判定失败（非对象/解析异常）回退 Markdown。 |
| D8 | 三态由 `ui.aiPanelMode` 驱动，`fullscreen` 不落 `localStorage`（`persist()` 直接 return），`restoredMode()` 只接受 `docked`/`hidden`。 |
| D9 | docked 与 fullscreen **共用一个 `<aside>` 元素**，只换 class/role；隐藏态才真正卸载面板。 |
| D10 | 未发送草稿放在 `ui.composerDraft`（store），面板用 `toRef(ui, 'composerDraft')` 读写；因此 D9 卸载后重建也不丢。 |
| D11 | 交接上下文放在 `ui.handoff`，面板顶部条显示 `traceId · verdict`，可跳回调查页或显式清除；一次性消费由用户点清除决定。 |
| D12 | 断点用 `useMediaQuery('(min-width: 1280px)')`；窄屏没有 docked 概念，`openAiPanel(isWide)` 在窄屏直接进 fullscreen。 |
| D13 | 全屏遮罩是**非模态** `role=dialog` + `aria-modal=false`：布局刻意让出侧栏（`left-16`）且没有焦点陷阱，声明成模态会让读屏把侧栏一起隐藏。打开时焦点进入面板，窄屏关闭时焦点回到悬浮按钮。 |

## 4. 数据流

```
SSE /api/chat (AG-UI events)         SQLite session events (replay)
              |                                     |
              v                                     v
        useAgUiStream                   messagesFromSessionEvents
 applyAgUiEvent(segments, e)               same Segment types
              |                                     |
              +------------------+------------------+
                                 v
                      chat store: messages[]
                                 |
                    +------------+------------+
                    v                         v
              MessageBubble             SessionsView
         thinking | text | tool       read-only replay
                    |
        +-----------+-----------+
        v           v           v
    Thinking    Markdown    ToolCall
      Block       Block       Card
                    |
 sanitizeMarkdown (marked -> DOMPurify)
```

布局侧：`AppShell` 读 `ui.aiPanelMode` + `useMediaQuery` 决定 `<aside>` 的 class 与是否挂载；`FailureDrawer` 写 `ui.handoff` 并把调查结论作为一次追问发给 `chat`。

## 5. 实现要点

### 5.1 有序段归约

`src/lib/chatModel.ts:26` 定义 `Segment` 联合、`:28` 定义 `ChatMessage`，`src/lib/chatModel.ts:97` 是事件 reducer。三条容易被写错的规则都有测试钉住：

- `TEXT_MESSAGE_START` 不带 delta，只负责**占位**，后续 `*_CONTENT` 才追加，否则「先 start 后 content」会丢顺序。
- 只有 `REASONING_MESSAGE_*` 建思考段；`REASONING_START/END` 携带的是另一个 `*-reasoning-phase-*` id，按它建段会切出空段。
- `TOOL_CALL_RESULT` 可能指向本面板从未见过的调用（挂起后在下一个 run 恢复），此时重建该段而不是丢弃 payload。
- 未答的工具卡在 `chat.send()` 返回后统一收尾（D4）。放在 store 而不是 reducer，是因为「run 结束」有四种入口：`RUN_FINISHED`、`RUN_ERROR`、响应体提前关闭、以及追问抢占（`useAgUiStream.send()` 开头的 `retireActive()` 会 abort 上一个 run，被 abort 的那个 run 收不到任何结束事件）。

store 侧（`src/stores/chat.ts:51`）用 `reactive` 的 assistant 消息，并把 `assistant.segments` 作为**代理上的数组**交给流式回调；缓存一个普通数组引用会让界面停在半路。

### 5.2 Markdown 与 JSON

`src/lib/markdown.ts` 把净化集中成一处：`marked(gfm, breaks)` → `DOMPurify.sanitize`，标签/属性白名单，`ALLOWED_URI_REGEXP` 收紧到 `https?:|mailto:`（默认策略还放行 `tel:`/`sms:`/`ftp:`/相对链接），`ALLOW_DATA_ATTR: false`，`afterSanitizeAttributes` 钩子给外链补 `target=_blank` + `rel=noopener noreferrer`——`mailto:` 例外：它该交给邮件客户端，新开一个标签页只会是一张空白页。

钩子必须在模块加载时注册：`addHook` 作用于 DOMPurify 单例，写在组件 `setup` 里会每挂载一个 `MarkdownBlock` 就叠一个钩子。

### 5.3 三态面板不丢状态

`src/components/AppShell.vue:53` 起，docked 与 fullscreen 是同一个 `<aside>`：切模式只改 class（`w-[420px] …` ↔ `fixed inset-y-0 left-16 right-0 z-50 …`）与 `role/aria-modal`，组件实例、DOM 节点、滚动位置全部保留。写成两个 `v-if` 分支时它们位于 children 数组的不同位置，Vue 会卸载再挂载：面板实例重建、局部 `ref` 归零，阅读位置与展开态一起消失。

这条在草稿搬进 store（D10）之后特别容易被测试放过：重挂载的新 `<textarea>` 会从 store 里读到同一份草稿，值看起来仍然正确。所以回归钉断言的是**节点同一**（`expect(overlay.element).toBe(asideBefore)`），实测它在两分支实现上失败、在单元素实现上通过。

`hidden` 仍然卸载面板，所以草稿必须在面板之外（D10）。滚动位置不跨隐藏保留——跟随流式的 watcher 带 `immediate`，重开面板即落到底部看到最新回答，是可接受行为，不为它加状态。

全屏态的可达性（D13）：`left-16` 让出侧栏宽度（`w-16` = 64px），`nav` 始终在遮罩之外可聚焦，因此面板声明 `aria-modal=false`；`role=dialog` 需要可访问名，`aria-label="AI Copilot"` 常驻两种模式。`Escape` 由 `AppShell` 的 window 监听处理（阅读者可能已经把焦点移到侧栏或看板去对照），`AiPanel` 自己只保留 composer 的 ⌘/Ctrl+Enter。窄屏关闭面板会卸载当前持有焦点的元素，`showFloatingButton` 的 watcher 把焦点交还给悬浮按钮。

### 5.4 失败→追问交接

`src/components/FailureDrawer.vue:225` 的 `handoff()` 依次做四件事：`ui.rememberHandoff({traceId, verdict, summary, windowMinutes})` 记下来源、`ui.openAiPanel(isWide)` 保证面板可见、关闭抽屉、把 `handoffPrompt` 作为一次追问发出去（追加在当前 transcript 之后；如果上一轮还在流式，这一发会抢占它，并按 D4 收尾上一轮未答的工具卡）。`handoffPrompt` 是从证据与分析结果拼出的结构化提问（trace_id、证据 fingerprint、结论类别与摘要、analysis_id；分析失败则带上错误原因），不是让用户手抄。抽屉必须关掉：一次分析可能跑上一分钟甚至失败，把用户扣在抽屉里没有意义。面板顶部条（`data-testid="handoff-context"`）显示来源并可跳回：`router.push({name:'failure', params:{traceId}, query:{windowMinutes}})`，分享链接因此能还原同一次调查。

## 6. 验证

- 单测：`npx vitest run` → 20 files / 133 tests 通过。新增 `chatModel.spec`(10)、`markdown.spec`(5)、`MarkdownBlock.spec`(5)、`MessageBubble.spec`(4)、`ThinkingBlock.spec`(4)、`ToolCallCard.spec`(3)、`ui.spec`(4)，并把面板/壳层的既有断言迁到段模型上（`chat.spec` 9、`AiPanel.spec` 7、`AppShell.spec` 10）。
- 回归钉：`AppShell.spec` 的 `keeps the draft when the panel changes size`（断言 `<aside>`/`<textarea>` 节点同一）、`keeps the draft across hide and reopen`、`returns focus to the opener when the narrow overlay closes`、以及 `leaves the side rail reachable…` 里的 `aria-modal=false` 断言。前两条分别在两分支实现与「草稿仍是组件局部 `ref`」的实现上失败过，第三条在只声明模态、不回收焦点的实现上失败过——都是先看红再实现。评审后的第二轮：`chat.spec` 的三条 settle 钉（`RUN_FINISHED` 未答、流提前关闭、被追问抢占）都在「只在 `RUN_ERROR` 和 `stop()` 收尾」的实现上红过；`AiPanel.spec` 的 `opens with the newest answer in view` 在没有 `immediate` 的 watcher 上红过；`markdown.spec` 的 `opens a mailto in the mail client, not a blank tab` 在统一补 `target=_blank` 的钩子上红过。
- 静态检查：`npm run type-check`、`npm run lint`（oxlint + eslint）均无告警。
- 浏览器实测（vite dev + 真实 console 后端 8787）：
  - 1440×900 docked → 全屏：`<aside>` 与 `<textarea>` 节点同一（`sameNode: true`），草稿保留，`role=dialog`、`aria-modal=false`，遮罩左边界 64px 等于侧栏宽度（`railRight: 64`），`localStorage` 仍为 `docked`。
  - `Escape` → 回到 docked，草稿保留。
  - 隐藏 → 悬浮按钮 → 重开：textarea 是**新节点**（`freshNode: true`）但草稿恢复，证明状态在 store 而非组件；`chat-list` 同样是新节点，重开后落在底部（`scrollTop 453 = scrollHeight 957 − clientHeight 504`），验证 `immediate` 跟随。
  - 1000×800（窄屏）悬浮按钮 → 打开遮罩：焦点进入面板（`document.activeElement === aside`）；`Escape` 关闭后面板卸载，焦点回到悬浮按钮（`aria-label="Open AI Copilot"`）。
  - 真实追问（一次 20 个工具调用的调查）：33 个段按流式顺序落位（`thinking,tool×3,thinking,tool×2,…,text`），20 张工具卡全部 `done`、收尾逻辑没有误伤已答卡片；正文 `##` 渲染成真实 `<h2>`；`chat.load()` 回放同一 session 得到**完全相同**的段序列（D5 的在线/回放一致）。
  - 链接策略：`mailto:` 不带 `target`/`rel`（交给邮件客户端），`https:` 带 `target=_blank` + `rel=noopener noreferrer`。

## 7. 已知限制

- 面板隐藏期间的流式滚动跟随不保留（重开回到底部）。
- 草稿只活到页面刷新；跨重启恢复需要给 `ui` store 加持久化，本期不做。
- 会话列表与回放未做虚拟化（实测 9–74 事件/会话，尚无瓶颈）；`contentSize` 只测最后一条消息，长回答的跟随仍然准确，但超大工具结果渲染仍是主线程成本。
- `useConnections` 每 30s 轮询一次健康；判定为 down 时输入框整体禁用（沿用改造前契约），此时草稿虽不丢但不能编辑。
- 抽屉与全屏面板各自监听 window `Escape`：两者同时打开时一次按键会同时关抽屉、退回 docked。要改成「只关最上层」需要一处协调，本期只记录。
- 发出问题后，草稿要等整个 run 结束才清空（沿用改造前契约：会话创建失败时不能把用户的问题吞掉，所以清除点在 `chat.send()` 返回之后）。实测一次 90 秒的调查里，输入框会显示已经发出去的那句话——改造前也如此，本期不改语义。
