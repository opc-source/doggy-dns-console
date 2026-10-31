<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, toRef, watch } from 'vue'
import { Copy, Send, TriangleAlert, X } from '@lucide/vue'

import type { MissingEvidence } from '@/api/types'
import { useInvestigation } from '@/composables/useInvestigation'
import { useChatStore } from '@/stores/chat'

const props = defineProps<{ traceId: string; windowMinutes: number; analysisId?: string }>()
const emit = defineEmits<{ close: [] }>()
const chat = useChatStore()

const trace = toRef(props, 'traceId')
const windowMinutes = toRef(props, 'windowMinutes')
const analysisId = toRef(props, 'analysisId')
const { state, evidence, analysis, error, stale, loadEvidence, startAnalysis, loadAnalysis } =
  useInvestigation(trace, windowMinutes)

type TabKey = 'logs' | 'spans' | 'instance' | 'metrics'

const tabs: Array<{ key: TabKey; label: string }> = [
  { key: 'logs', label: '日志' },
  { key: 'spans', label: 'Span' },
  { key: 'instance', label: '实例' },
  { key: 'metrics', label: '指标' },
]

const copied = ref(false)
const closeButton = ref<HTMLButtonElement | null>(null)
const dialog = ref<HTMLElement | null>(null)
const evidenceBody = ref<HTMLElement | null>(null)
const activeTab = ref<TabKey>('logs')
const highlightedId = ref<string | null>(null)
const evidenceFailed = ref(false)
let highlightTimer: ReturnType<typeof setTimeout> | null = null
let loadSeq = 0
let previousFocus: Element | null = null

const title = computed(() => evidence.value?.group.query ?? props.traceId)
const group = computed(() => evidence.value?.group ?? null)
const logs = computed(() => evidence.value?.logs ?? null)
const spans = computed(() => evidence.value?.spans ?? null)
const instance = computed(() => evidence.value?.instance ?? null)
const metrics = computed(() => evidence.value?.metrics ?? null)
const host = computed(() => metrics.value?.host ?? null)
const config = computed(() => instance.value?.config ?? null)
const result = computed(() => analysis.value?.result ?? null)

const stateLabel = computed(() => {
  switch (state.value) {
    case 'evidenceLoading':
      return '取证中'
    case 'evidenceReady':
      return '证据就绪'
    case 'analysisQueued':
      return '排队中'
    case 'analysisRunning':
      return '分析中'
    case 'analysisDone':
      if (result.value?.status === 'unavailable') return '不可用'
      return result.value?.status === 'insufficient_evidence' ? '证据不足' : '已完成'
    case 'analysisUnavailable':
      return '不可用'
    case 'stale':
      return '已过期'
    default:
      return ''
  }
})

const resultTag = computed(() => {
  switch (result.value?.status) {
    case 'ok':
      return '结论'
    case 'insufficient_evidence':
      return '证据不足'
    case 'unavailable':
      return '不可用'
    case 'invalid':
      return '结果非法'
    default:
      return ''
  }
})

const showCardReanalyze = computed(
  () =>
    !stale.value &&
    result.value !== null &&
    (state.value === 'analysisDone' || state.value === 'analysisUnavailable'),
)

const findingSections = computed(() => {
  const findings = result.value?.findings ?? []
  return [
    { key: 'observed', label: '观测', items: findings.filter((item) => item.kind === 'observed') },
    { key: 'inferred', label: '推断', items: findings.filter((item) => item.kind === 'inferred') },
  ].filter((section) => section.items.length > 0)
})

const evidenceError = computed(() => (evidenceFailed.value ? (error.value ?? '取证失败') : null))

const failedSnapshot = computed(
  () => analysis.value?.status === 'failed' || analysis.value?.status === 'invalid',
)

const analysisError = computed(() => {
  if (state.value !== 'analysisUnavailable' && !failedSnapshot.value) return null
  return analysis.value?.error ?? error.value ?? '分析失败，请稍后重试'
})

// A 429/transient start failure settles on `evidenceReady`, and a failed poll
// leaves `analysisRunning`; both keep `error` set but have no dedicated banner.
// Reload states are excluded so an old error cannot flash over fresh evidence.
const genericAnalysisError = computed(() => {
  if (error.value === null || evidenceFailed.value) return null
  if (state.value === 'analysisUnavailable' || failedSnapshot.value) return null
  if (state.value === 'evidenceLoading' || state.value === 'idle') return null
  return error.value
})

const MISSING_LABELS: Record<MissingEvidence, string> = {
  instance_unreachable: '实例不可达',
  instance_not_found: '未找到实例',
  spans_unavailable: 'Span 不可用',
  metrics_unavailable: '指标不可用',
  logs_truncated: '日志已截断',
}

const handoffPrompt = computed(() => {
  const lines = [`请继续调查这组 DNS 失败：trace_id=${props.traceId}`]
  if (evidence.value) lines.push(`fingerprint=${evidence.value.fingerprint}`)
  if (result.value) {
    lines.push(`结论=${result.value.verdict.category}`, `摘要=${result.value.verdict.summary}`)
  } else {
    const reason = analysis.value?.error ?? error.value
    if (reason) lines.push(`错误=${reason}`)
  }
  if (analysis.value) lines.push(`analysis_id=${analysis.value.analysisId}`)
  return lines.join('\n')
})

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    emit('close')
    return
  }
  if (event.key !== 'Tab' || !dialog.value) return
  const focusable = Array.from(
    dialog.value.querySelectorAll<HTMLElement>(
      'button, [href], input, textarea, select, [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => !element.hasAttribute('disabled'))
  if (focusable.length === 0) return
  const first = focusable[0]
  const last = focusable[focusable.length - 1]
  if (!first || !last) return
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}

async function investigate() {
  const seq = ++loadSeq
  evidenceFailed.value = false
  const ok = await loadEvidence()
  // A stale invocation (trace/window changed or a reload started meanwhile) must
  // not paint its own outcome over the newest one.
  if (seq !== loadSeq) return
  evidenceFailed.value = !ok
  if (!ok) return

  // A shared deep link restores the named attempt instead of starting a new one.
  if (analysisId.value) {
    const restored = await loadAnalysis(analysisId.value)
    if (seq !== loadSeq) return
    // `loadAnalysis` returns false after a silent 404 with `error` still null;
    // only then is a fresh attempt the right fallback.
    if (!restored && error.value === null) await startAnalysis()
    return
  }

  if (state.value === 'evidenceReady') await startAnalysis()
}

onMounted(() => {
  previousFocus = document.activeElement
  window.addEventListener('keydown', onKeydown)
  void nextTick(() => closeButton.value?.focus())
  void investigate()
})

watch([trace, windowMinutes, analysisId], () => {
  void investigate()
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
  if (highlightTimer !== null) clearTimeout(highlightTimer)
  if (previousFocus instanceof HTMLElement) previousFocus.focus()
})

async function copyTrace() {
  try {
    await navigator.clipboard.writeText(props.traceId)
    copied.value = true
    setTimeout(() => (copied.value = false), 1200)
  } catch {
    copied.value = false
  }
}

async function handoff() {
  const sent = await chat.send(handoffPrompt.value)
  if (sent) emit('close')
}

function when(ts: number | null | undefined) {
  return ts === null || ts === undefined ? '—' : new Date(ts).toLocaleString()
}

function missingLabel(value: MissingEvidence) {
  return MISSING_LABELS[value] ?? value
}

function severityClass(severity: string) {
  const level = severity.toUpperCase()
  if (level === 'ERROR' || level === 'FATAL') return 'bg-error/15 text-error'
  if (level === 'WARN' || level === 'WARNING') return 'bg-warn/15 text-warn'
  return 'bg-surface-2 text-muted'
}

function rowClass(id: string) {
  return id === highlightedId.value
    ? 'is-cited border-accent/50 bg-accent/5'
    : 'border-border bg-surface-2/50'
}

function tabForId(id: string): TabKey | null {
  const prefix = id.slice(0, id.indexOf(':'))
  if (prefix === 'log') return 'logs'
  if (prefix === 'span') return 'spans'
  if (prefix === 'plugin' || prefix === 'config') return 'instance'
  if (prefix === 'metric') return 'metrics'
  return null
}

function findRow(id: string): HTMLElement | null {
  if (!evidenceBody.value) return null
  const rows = Array.from(evidenceBody.value.querySelectorAll<HTMLElement>('[data-evidence-id]'))
  return rows.find((row) => row.dataset.evidenceId === id) ?? null
}

function revealEvidence(id: string) {
  const tab = tabForId(id)
  if (!tab) return
  activeTab.value = tab
  highlightedId.value = id
  if (highlightTimer !== null) clearTimeout(highlightTimer)
  highlightTimer = setTimeout(() => {
    highlightedId.value = null
    highlightTimer = null
  }, 2000)
  void nextTick(() => {
    findRow(id)?.scrollIntoView?.({ block: 'center', behavior: 'smooth' })
  })
}
</script>

<template>
  <div class="fixed inset-0 z-40 flex justify-end bg-black/40 backdrop-blur-[2px]" @click.self="emit('close')">
    <aside
      ref="dialog"
      class="flex h-full w-full max-w-[480px] flex-col border-l border-border bg-surface shadow-2xl"
      data-testid="failure-drawer"
      role="dialog"
      aria-modal="true"
      aria-label="Failure detail"
    >
      <header class="border-b border-border px-4 py-4">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <p class="mono text-[10px] uppercase tracking-[0.16em] text-error">failure trace</p>
            <h2 class="mono mt-1 truncate text-sm text-text">{{ title }}</h2>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <button
              class="focus-ring mono flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-[11px] text-muted hover:text-text"
              @click="copyTrace"
            >
              <Copy :size="11" /> {{ copied ? 'copied' : props.traceId }}
            </button>
            <button
              ref="closeButton"
              class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border text-muted hover:text-text"
              aria-label="Close"
              @click="emit('close')"
            >
              <X :size="15" />
            </button>
          </div>
        </div>
        <div class="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-muted">
          <span v-if="group" class="rounded-md bg-error/15 px-1.5 py-0.5 text-error">{{ group.responseCode }}</span>
          <span v-if="group" class="tabular-nums">×{{ group.count }}</span>
          <span v-if="group?.instance" class="mono">{{ group.instance }}</span>
          <span
            v-if="stateLabel"
            data-testid="investigation-state"
            class="rounded-full border border-border px-2 py-0.5 text-[10px] text-muted"
          >
            {{ stateLabel }}
          </span>
        </div>
      </header>

      <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-4" data-testid="investigation-panel">
        <p
          v-if="evidenceError"
          class="flex items-start gap-2 rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"
        >
          <TriangleAlert :size="14" class="mt-0.5 shrink-0" />
          <span class="min-w-0 flex-1 break-all">{{ evidenceError }}</span>
          <button
            data-test="reload-evidence"
            class="focus-ring shrink-0 rounded-lg border border-error/40 px-2 py-1 text-[11px] hover:bg-error/10"
            @click="investigate()"
          >
            重新取证
          </button>
        </p>

        <p
          v-if="stale"
          class="flex items-start gap-2 rounded-lg border border-warn/30 bg-warn/10 px-3 py-2 text-xs text-warn"
        >
          <TriangleAlert :size="14" class="mt-0.5 shrink-0" />
          <span class="min-w-0 flex-1">证据已变化，需重新分析</span>
          <button
            data-test="reanalyze"
            class="focus-ring shrink-0 rounded-lg border border-warn/40 px-2 py-1 text-[11px] hover:bg-warn/10"
            @click="startAnalysis(true)"
          >
            重新分析
          </button>
        </p>

        <section v-if="result" data-testid="verdict-card" class="panel space-y-3 p-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="mono text-[10px] uppercase tracking-[0.14em] text-muted">结论</span>
            <span class="rounded-full border border-accent/30 px-2 py-0.5 text-[10px] text-accent">{{ resultTag }}</span>
            <span class="mono text-sm text-text">{{ result.verdict.category }}</span>
            <span class="rounded-full border border-border px-2 py-0.5 text-[10px] text-muted">
              置信度 {{ result.verdict.confidence }}
            </span>
            <button
              v-if="showCardReanalyze"
              data-test="reanalyze"
              class="focus-ring mono ml-auto rounded-lg border border-border px-2 py-1 text-[10px] text-muted hover:text-text"
              @click="startAnalysis(true)"
            >
              重新分析
            </button>
          </div>
          <p class="text-xs leading-relaxed text-text">{{ result.verdict.summary }}</p>
          <div v-if="result.notDeterminable.length" class="rounded-lg border border-border bg-surface-2/50 p-2">
            <p class="text-[10px] uppercase tracking-[0.12em] text-muted">无法确定</p>
            <ul class="mt-1 list-disc space-y-0.5 pl-4 text-[11px] text-muted">
              <li v-for="item in result.notDeterminable" :key="item">{{ item }}</li>
            </ul>
          </div>
        </section>

        <p
          v-if="analysisError"
          class="flex items-start gap-2 rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"
        >
          <TriangleAlert :size="14" class="mt-0.5 shrink-0" />
          <span class="min-w-0 flex-1 break-all">{{ analysisError }}</span>
          <button
            data-test="retry-analysis"
            class="focus-ring shrink-0 rounded-lg border border-error/40 px-2 py-1 text-[11px] hover:bg-error/10"
            @click="startAnalysis()"
          >
            重试分析
          </button>
        </p>

        <p
          v-if="genericAnalysisError"
          class="flex items-start gap-2 rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"
        >
          <TriangleAlert :size="14" class="mt-0.5 shrink-0" />
          <span class="min-w-0 flex-1 break-all">{{ genericAnalysisError }}</span>
          <button
            data-test="retry-analysis"
            class="focus-ring shrink-0 rounded-lg border border-error/40 px-2 py-1 text-[11px] hover:bg-error/10"
            @click="startAnalysis()"
          >
            重试分析
          </button>
        </p>

        <section
          v-for="section in findingSections"
          :key="section.key"
          :data-testid="`finding-${section.key}`"
          class="space-y-2"
        >
          <h3 class="text-[11px] uppercase tracking-[0.14em] text-muted">{{ section.label }}</h3>
          <article
            v-for="(finding, index) in section.items"
            :key="`${section.key}-${index}`"
            class="rounded-xl border border-border bg-surface-2/50 p-3"
          >
            <p class="text-xs text-text">{{ finding.claim }}</p>
            <div class="mt-2 flex flex-wrap gap-1.5">
              <button
                v-for="id in finding.evidenceIds"
                :key="id"
                data-test="evidence-ref"
                :data-evidence-id="id"
                class="focus-ring mono rounded-md border border-border px-1.5 py-0.5 text-[10px] text-muted hover:border-accent/40 hover:text-accent"
                @click="revealEvidence(id)"
              >
                {{ id }}
              </button>
            </div>
          </article>
        </section>

        <section v-if="evidence" class="space-y-2">
          <div class="flex items-center gap-1" role="tablist">
            <button
              v-for="tab in tabs"
              :key="tab.key"
              role="tab"
              :aria-selected="activeTab === tab.key"
              :data-test="`tab-${tab.key}`"
              class="focus-ring rounded-lg border px-2.5 py-1 text-[11px]"
              :class="
                activeTab === tab.key
                  ? 'border-accent/30 bg-accent/10 text-text'
                  : 'border-transparent text-muted hover:text-text'
              "
              @click="activeTab = tab.key"
            >
              {{ tab.label }}
            </button>
          </div>

          <ul v-if="evidence.missing.length" class="flex flex-wrap gap-1.5">
            <li
              v-for="item in evidence.missing"
              :key="item"
              class="rounded-full border border-warn/30 px-2 py-0.5 text-[10px] text-warn"
            >
              {{ missingLabel(item) }}
            </li>
          </ul>

          <div ref="evidenceBody" class="space-y-2" role="tabpanel">
            <template v-if="activeTab === 'logs' && logs">
              <div class="flex flex-wrap items-center gap-2 text-[10px] text-muted">
                <span class="tabular-nums">共 {{ logs.total }} 条</span>
                <span v-if="logs.omitted" class="rounded bg-surface-2 px-1.5 py-0.5">已省略 {{ logs.omitted }} 条</span>
                <span v-if="logs.truncated" data-test="logs-truncated" class="rounded bg-warn/15 px-1.5 py-0.5 text-warn">已截断</span>
              </div>
              <article
                v-for="log in logs.items"
                :key="log.id"
                :data-evidence-id="log.id"
                class="rounded-xl border p-3 transition-colors"
                :class="rowClass(log.id)"
              >
                <div class="flex flex-wrap items-center gap-2 text-[10px]">
                  <span class="mono text-muted">{{ log.id }}</span>
                  <span class="rounded px-1.5 py-0.5" :class="severityClass(log.severity)">{{ log.severity }}</span>
                  <span class="tabular-nums text-muted">{{ when(log.ts) }}</span>
                  <span v-if="log.spanId" class="mono text-muted">span={{ log.spanId }}</span>
                </div>
                <p class="mt-2 text-xs text-text">{{ log.message }}</p>
                <div
                  v-if="log.query || log.qtype || log.responseCode || log.error"
                  class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted"
                >
                  <span v-if="log.query" class="mono">query={{ log.query }}</span>
                  <span v-if="log.qtype" class="mono">qtype={{ log.qtype }}</span>
                  <span v-if="log.responseCode" class="mono">rcode={{ log.responseCode }}</span>
                  <span v-if="log.error" class="text-error">error={{ log.error }}</span>
                </div>
              </article>
              <p v-if="logs.items.length === 0" class="text-xs text-muted">暂无日志</p>
            </template>

            <template v-else-if="activeTab === 'spans' && spans">
              <div class="flex flex-wrap items-center gap-2 text-[10px] text-muted">
                <span class="tabular-nums">共 {{ spans.total }} 条</span>
                <span v-if="spans.truncated" class="rounded bg-warn/15 px-1.5 py-0.5 text-warn">已截断</span>
              </div>
              <article
                v-for="span in spans.items"
                :key="span.id"
                :data-evidence-id="span.id"
                class="rounded-xl border p-3 transition-colors"
                :class="rowClass(span.id)"
              >
                <div class="flex items-center gap-2">
                  <span class="mono text-[10px] text-muted">{{ span.id }}</span>
                  <span class="mono truncate text-xs text-text">{{ span.name }}</span>
                  <span class="ml-auto shrink-0 tabular-nums text-[10px] text-muted">
                    {{ span.durationMs === null ? '—' : `${span.durationMs} ms` }}
                  </span>
                </div>
                <p class="mono mt-1 text-[10px] text-muted">{{ span.spanId }} · {{ span.status }}</p>
              </article>
              <p v-if="spans.items.length === 0" class="text-xs text-muted">暂无 Span</p>
            </template>

            <template v-else-if="activeTab === 'instance'">
              <p v-if="!instance" class="rounded-xl border border-border bg-surface-2/50 p-3 text-xs text-muted">
                未匹配到实例
              </p>
              <template v-else>
                <div class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <div class="flex flex-wrap items-center gap-2 text-[10px]">
                    <span class="mono text-muted">{{ instance.matchQuality }}</span>
                    <span :class="instance.reachable ? 'text-ok' : 'text-error'">
                      {{ instance.reachable ? '可访问' : '不可访问' }}
                    </span>
                    <span v-if="instance.status" class="mono text-muted">{{ instance.status }}</span>
                  </div>
                  <dl class="mt-2 grid grid-cols-[88px_1fr] gap-x-2 gap-y-1 text-[11px]">
                    <template v-if="instance.identity.serviceInstanceId">
                      <dt class="text-muted">实例 ID</dt>
                      <dd class="mono break-all text-text">{{ instance.identity.serviceInstanceId }}</dd>
                    </template>
                    <template v-if="instance.identity.host">
                      <dt class="text-muted">主机</dt>
                      <dd class="mono break-all text-text">{{ instance.identity.host }}</dd>
                    </template>
                    <template v-if="instance.identity.ip">
                      <dt class="text-muted">IP</dt>
                      <dd class="mono break-all text-text">{{ instance.identity.ip }}</dd>
                    </template>
                    <template v-if="instance.pid !== null">
                      <dt class="text-muted">pid</dt>
                      <dd class="tabular-nums text-text">{{ instance.pid }}</dd>
                    </template>
                    <template v-if="instance.uptimeSeconds !== null">
                      <dt class="text-muted">运行时长</dt>
                      <dd class="tabular-nums text-text">{{ instance.uptimeSeconds }}s</dd>
                    </template>
                    <template v-if="instance.listenAddr">
                      <dt class="text-muted">监听地址</dt>
                      <dd class="mono break-all text-text">{{ instance.listenAddr }}</dd>
                    </template>
                  </dl>
                </div>

                <div class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <p class="text-[10px] uppercase tracking-[0.12em] text-muted">插件</p>
                  <ul class="mt-1 space-y-1">
                    <li
                      v-for="plugin in instance.plugins"
                      :key="plugin.id"
                      :data-evidence-id="plugin.id"
                      class="rounded-lg border px-2 py-1 text-[11px] text-muted transition-colors"
                      :class="rowClass(plugin.id)"
                    >
                      <span class="mono text-text">{{ plugin.id }}</span>
                      <span class="ml-1">
                        #{{ plugin.order }} {{ plugin.kind }} · {{ plugin.enabled ? 'enabled' : 'disabled' }}
                      </span>
                      <span class="mono ml-1 break-all">{{ JSON.stringify(plugin.details) }}</span>
                    </li>
                    <li v-if="instance.plugins.length === 0" class="text-[11px] text-muted">无插件信息</li>
                  </ul>
                </div>

                <div v-if="config" class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <p class="text-[10px] uppercase tracking-[0.12em] text-muted">
                    配置 <span class="mono normal-case">{{ config.id }}</span>
                  </p>
                  <pre
                    :data-evidence-id="config.id"
                    class="mono mt-2 max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-lg border p-2 text-[10px] text-text"
                    :class="rowClass(config.id)"
                  >{{ config.content }}</pre>
                  <p v-if="config.redactedKeys.length" class="mt-1 text-[10px] text-warn">
                    已脱敏：{{ config.redactedKeys.join('、') }}
                  </p>
                </div>

                <ul v-if="instance.missing.length" class="flex flex-wrap gap-1.5">
                  <li
                    v-for="item in instance.missing"
                    :key="item"
                    class="rounded-full border border-warn/30 px-2 py-0.5 text-[10px] text-warn"
                  >
                    {{ missingLabel(item) }}
                  </li>
                </ul>
              </template>
            </template>

            <template v-else-if="activeTab === 'metrics'">
              <p v-if="!metrics" class="rounded-xl border border-border bg-surface-2/50 p-3 text-xs text-muted">
                指标不可用
              </p>
              <template v-else>
                <p class="tabular-nums text-[10px] text-muted">
                  {{ when(metrics.range.start) }} → {{ when(metrics.range.end) }}
                </p>
                <div
                  :data-evidence-id="metrics.queriesId"
                  class="flex items-center justify-between rounded-xl border px-3 py-2 text-xs transition-colors"
                  :class="rowClass(metrics.queriesId)"
                >
                  <span class="text-muted">全局查询</span>
                  <span class="tabular-nums text-text">{{ metrics.global.queries }}</span>
                </div>
                <div
                  :data-evidence-id="metrics.failuresId"
                  class="flex items-center justify-between rounded-xl border px-3 py-2 text-xs transition-colors"
                  :class="rowClass(metrics.failuresId)"
                >
                  <span class="text-muted">全局失败</span>
                  <span class="tabular-nums text-text">{{ metrics.global.failures }}</span>
                </div>
                <div v-if="host" class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <p class="mono truncate text-[11px] text-text">{{ host.name }}</p>
                  <div
                    :data-evidence-id="host.queriesId"
                    class="mt-1 flex items-center justify-between rounded-lg border px-2 py-1 text-[11px] transition-colors"
                    :class="rowClass(host.queriesId)"
                  >
                    <span class="text-muted">主机查询</span>
                    <span class="tabular-nums text-text">{{ host.queries }}</span>
                  </div>
                  <div
                    :data-evidence-id="host.failuresId"
                    class="mt-1 flex items-center justify-between rounded-lg border px-2 py-1 text-[11px] transition-colors"
                    :class="rowClass(host.failuresId)"
                  >
                    <span class="text-muted">主机失败</span>
                    <span class="tabular-nums text-text">{{ host.failures }}</span>
                  </div>
                </div>
                <p v-else class="text-[11px] text-muted">无主机级指标</p>
              </template>
            </template>
          </div>
        </section>

        <details v-if="analysis" data-testid="forensics" class="panel px-3 py-2">
          <summary class="focus-ring cursor-pointer text-[11px] text-muted">取证过程</summary>
          <dl class="mt-2 space-y-1 text-[11px]">
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">尝试次数</dt>
              <dd data-test="forensics-attempt" class="tabular-nums text-text">{{ analysis.attempt }}</dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">工具调用</dt>
              <dd class="tabular-nums text-text">{{ analysis.toolCallCount }}</dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">指纹</dt>
              <dd data-test="forensics-fingerprint" class="mono break-all text-text">
                {{ analysis.evidenceFingerprint }}
              </dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">模型</dt>
              <dd data-test="forensics-model" class="mono break-all text-text">{{ analysis.model }}</dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">提示版本</dt>
              <dd data-test="forensics-prompt-version" class="mono text-text">{{ analysis.promptVersion }}</dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">开始</dt>
              <dd class="tabular-nums text-text">{{ when(analysis.startedAt) }}</dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-20 shrink-0 text-muted">结束</dt>
              <dd class="tabular-nums text-text">{{ when(analysis.finishedAt) }}</dd>
            </div>
          </dl>
          <div v-if="analysis.toolCalls.length" class="mt-2 space-y-1">
            <p class="text-[10px] uppercase tracking-[0.12em] text-muted">工具调用明细</p>
            <div
              v-for="(call, index) in analysis.toolCalls"
              :key="`${call.name}-${index}`"
              class="rounded-lg border border-border bg-surface-2/50 p-2 text-[10px]"
            >
              <div class="flex flex-wrap items-center gap-2">
                <span class="mono text-text">{{ call.name }}</span>
                <span class="tabular-nums text-muted">{{ call.durationMs }} ms</span>
                <span class="mono break-all text-muted">{{ call.resultHash }}</span>
              </div>
              <p class="mt-1 break-all text-muted">{{ call.summary }}</p>
            </div>
          </div>
        </details>
      </div>

      <footer class="border-t border-border p-4">
        <button
          data-test="handoff"
          class="focus-ring flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-2.5 text-xs font-semibold text-bg hover:opacity-90 disabled:opacity-40"
          @click="handoff"
        >
          <Send :size="14" /> 继续追问
        </button>
      </footer>
    </aside>
  </div>
</template>
