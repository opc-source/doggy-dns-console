<script setup lang="ts">
import { computed, ref, toRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RefreshCw, X } from '@lucide/vue'

import ConfigPanel from '@/components/ConfigPanel.vue'
import ErrorBanner from '@/components/ErrorBanner.vue'
import InstanceCard from '@/components/InstanceCard.vue'
import StatusDot from '@/components/StatusDot.vue'
import TimeRangeControl from '@/components/TimeRangeControl.vue'
import { apiPost } from '@/api/http'
import { useInstanceDetail, useInstances } from '@/composables/useInstances'
import type { Instance, LogLevelState } from '@/api/types'
import {
  DEFAULT_WINDOW_MINUTES,
  parseFilterQuery,
  relativeDraft,
  toRouteQuery,
  type FilterDraft,
} from '@/lib/failureFilter'

const route = useRoute()
const router = useRouter()

// `route.query` is a `LocationQuery`,数组成员可能为 null;`parseFilterQuery` 只取
// 第一个字符串,所以两边运行时对得上,只需要一处不加宽度的转换。
type FilterQuery = Parameters<typeof parseFilterQuery>[0]

// 与 `OverviewView.vue` 同款:URL 是唯一状态来源,draft 从路由派生,写入只回到
// 路由。一条时间参数都没有时补 `relativeDraft(DEFAULT_WINDOW_MINUTES)`,抽屉就
// 永远带一个显式窗口去请求;抽屉开合是本地 `selected`,不是区间语义,不进 URL。
const draft = computed(() => {
  const parsed = parseFilterQuery(route.query as FilterQuery)
  if (parsed.fromMs === null && parsed.toMs === null && parsed.windowMinutes === null) {
    return relativeDraft(DEFAULT_WINDOW_MINUTES)
  }
  return parsed
})

function setDraft(next: FilterDraft) {
  void router.replace({ query: toRouteQuery(next) })
}

const { instances, error, refresh } = useInstances()
const selected = ref<string | null>(null)
const selectedId = toRef(selected)
const { detail, error: detailError, invalid, loading, refresh: refreshDetail } = useInstanceDetail(selectedId, draft)

const logLevelDraft = ref('')
const logLevelBusy = ref(false)
const logLevelError = ref<string | null>(null)
let logLevelSeq = 0

watch(selected, () => {
  // The drawer moved: retire any answer still in flight so A's failure cannot
  // render under B, and release the spinner the closed panel will not show.
  logLevelSeq += 1
  logLevelDraft.value = ''
  logLevelError.value = null
  logLevelBusy.value = false
})

const logLevels = computed(() => detail.value?.runtime?.logs ?? null)
const canControl = computed(
  () =>
    Boolean(detail.value?.controlEnabled) &&
    Boolean(detail.value?.instance.online) &&
    Boolean(detail.value?.instance.mcpReachable),
)

async function changeLogLevel(payload: { directives?: string; reset?: boolean }) {
  const current = detail.value
  if (!current || logLevelBusy.value) return
  const seq = ++logLevelSeq
  logLevelBusy.value = true
  logLevelError.value = null
  try {
    await apiPost<LogLevelState>(
      `/api/instances/${encodeURIComponent(current.instance.id)}/log-level`,
      payload,
    )
    if (seq !== logLevelSeq) return
    logLevelDraft.value = ''
    await refreshDetail()
  } catch (cause) {
    if (seq !== logLevelSeq) return
    logLevelError.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (seq === logLevelSeq) logLevelBusy.value = false
  }
}

function applyDirectives() {
  const directives = logLevelDraft.value.trim()
  if (!directives) return
  void changeLogLevel({ directives })
}

function resetLogLevel() {
  void changeLogLevel({ reset: true })
}

const online = computed(() => instances.value.filter((item) => item.online).length)

function select(instance: Instance) {
  selected.value = instance.id
}

function describeSignal(signal: unknown): string | null {
  if (!signal || typeof signal !== 'object') return null
  const value = signal as Record<string, unknown>
  if (typeof value.cachedServices === 'number') {
    const push = value.lastPushSecondsAgo
    return `cached services ${value.cachedServices} · ${
      typeof push === 'number' ? `last push ${push}s ago` : 'no push yet'
    }`
  }
  if (typeof value.attached === 'boolean') {
    return value.attached ? 'fast path attached' : 'fast path detached'
  }
  return null
}
</script>

<template>
  <div class="mx-auto flex max-w-[1440px] flex-col gap-4 p-5 lg:p-7">
    <header class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="mono text-[11px] uppercase tracking-[0.18em] text-accent">fleet</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight lg:text-3xl">Instances</h1>
        <p class="mt-1 text-sm text-muted">{{ online }} online · {{ instances.length }} known</p>
      </div>
      <div class="flex items-center gap-2">
        <TimeRangeControl :model-value="draft" @update:model-value="setDraft" />
        <button class="focus-ring flex items-center gap-2 rounded-xl border border-border bg-surface px-3 py-2 text-xs text-muted hover:text-text" @click="refresh">
          <RefreshCw :size="13" /> Refresh
        </button>
      </div>
    </header>

    <ErrorBanner v-if="error" :message="error" @retry="refresh" />

    <div v-if="instances.length === 0" class="panel p-10 text-center">
      <p class="text-sm text-text">还没有发现 doggy-dns 实例</p>
      <p class="mt-1 text-xs text-muted">确认 OpenObserve 已收到 host.name 与 doggy_dns.ip_addr/control.port，或配置 MCP_ENDPOINTS 兜底。</p>
    </div>
    <section v-else class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      <InstanceCard v-for="instance in instances" :key="instance.id" :instance="instance" @select="select" />
    </section>

    <div
      v-if="selected"
      class="fixed inset-0 z-40 flex justify-end bg-black/40 backdrop-blur-[2px]"
      tabindex="-1"
      @click.self="selected = null"
      @keydown.esc="selected = null"
    >
      <aside
        class="flex h-full w-full max-w-[1200px] flex-col border-l border-border bg-surface shadow-2xl sm:w-[70%]"
        role="dialog"
        aria-modal="true"
        aria-label="Instance detail"
      >
        <header class="flex items-start justify-between border-b border-border px-4 py-4">
          <div>
            <p class="mono text-[10px] uppercase tracking-[0.16em] text-accent">instance</p>
            <h2 class="mono mt-1 text-sm text-text">{{ selected }}</h2>
            <div class="mt-2 flex items-center gap-2">
              <StatusDot :status="detail?.instance.online ? 'ok' : 'error'" :label="detail?.instance.online ? 'online' : 'offline'" />
              <span v-if="detail" class="mono text-[10px] text-muted">{{ detail.listenAddr }}</span>
            </div>
          </div>
          <button class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border text-muted hover:text-text" aria-label="Close" @click="selected = null">
            <X :size="15" />
          </button>
        </header>

        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
          <p v-if="loading" class="text-xs text-muted">Loading instance…</p>
          <p
            v-else-if="invalid"
            class="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error"
            data-testid="detail-invalid"
          >
            {{ invalid }}
          </p>
          <p v-else-if="detailError" class="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{{ detailError }}</p>
          <template v-else-if="detail">
            <section class="grid grid-cols-2 gap-2">
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">区间内失败</p>
                <p class="mono mt-1 text-sm" data-testid="failure-count">{{ detail.failureCount ?? '—' }}</p>
                <RouterLink
                  class="mono mt-1 inline-block text-[10px] text-accent"
                  data-testid="failures-for-host"
                  :to="{
                    name: 'failures-list',
                    query: { ...toRouteQuery(draft), instance: detail.instance.host },
                  }"
                  >在失败列表里看这台主机</RouterLink
                >
              </div>
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">uptime</p>
                <p class="mt-1 text-sm">{{ Math.floor((detail.uptimeSeconds ?? 0) / 3600) }}h</p>
              </div>
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">protocols</p>
                <p class="mono mt-1 text-sm">{{ detail.protocols.join(' · ') || '—' }}</p>
              </div>
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">status</p>
                <p class="mono mt-1 text-sm">{{ detail.status ?? '—' }}</p>
              </div>
              <div class="panel p-3">
                <p class="text-[10px] uppercase tracking-wide text-muted">pid</p>
                <p class="mono mt-1 text-sm">{{ detail.pid ?? '—' }}</p>
              </div>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Live signals</h3>
              <div
                v-if="detail.runtime && Object.keys(detail.runtime.signals).length"
                class="space-y-2"
              >
                <article
                  v-for="(signal, name) in detail.runtime.signals"
                  :key="name"
                  class="rounded-xl border border-border bg-surface-2/50 p-3"
                >
                  <p class="mono text-xs">{{ name }}</p>
                  <p v-if="describeSignal(signal)" class="mt-1 text-[11px] text-muted">
                    {{ describeSignal(signal) }}
                  </p>
                  <pre
                    v-else
                    class="mono mt-1 whitespace-pre-wrap break-all text-[10px] text-muted"
                  >{{ JSON.stringify(signal) }}</pre>
                </article>
              </div>
              <p v-else-if="detail.runtime" class="text-xs text-muted">No live signals registered.</p>
              <p v-else class="text-xs text-muted">MCP did not return runtime signals.</p>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Runtime log level</h3>
              <div class="panel p-3">
                <div class="grid grid-cols-2 gap-2">
                  <div>
                    <p class="text-[10px] uppercase tracking-wide text-muted">stdout</p>
                    <p class="mono mt-1 text-xs" data-testid="log-level-stdout">
                      {{ logLevels ? (logLevels.stdout ?? 'off') : '—' }}
                    </p>
                  </div>
                  <div>
                    <p class="text-[10px] uppercase tracking-wide text-muted">otlp</p>
                    <p class="mono mt-1 text-xs" data-testid="log-level-otlp">
                      {{ logLevels ? (logLevels.otlp ?? 'off') : '—' }}
                    </p>
                  </div>
                </div>
                <p v-if="!logLevels" class="mt-2 text-[11px] text-muted">
                  MCP did not return log filters.
                </p>
                <template v-if="canControl">
                  <div class="mt-3 flex flex-wrap items-center gap-2">
                    <input
                      v-model="logLevelDraft"
                      aria-label="Log filter directives"
                      data-testid="log-level-input"
                      class="mono min-w-0 flex-1 rounded-lg border border-border bg-surface-2 px-3 py-2 text-xs"
                      placeholder="doggy_dns_nacos=debug,hickory_proto=info"
                      :disabled="logLevelBusy"
                      @keydown.enter="applyDirectives"
                    />
                    <button
                      data-testid="log-level-apply"
                      class="focus-ring rounded-lg border border-border bg-surface px-3 py-2 text-xs text-text disabled:opacity-50"
                      :disabled="logLevelBusy || !logLevelDraft.trim()"
                      @click="applyDirectives"
                    >
                      Apply
                    </button>
                    <button
                      data-testid="log-level-reset"
                      class="focus-ring rounded-lg border border-border bg-surface px-3 py-2 text-xs text-muted hover:text-text disabled:opacity-50"
                      :disabled="logLevelBusy"
                      @click="resetLogLevel"
                    >
                      Reset
                    </button>
                  </div>
                  <p class="mt-2 text-[11px] text-muted">
                    Applies to stdout and OTLP until reset or restart; it does not write the config
                    file.
                  </p>
                </template>
                <p v-else-if="!detail.controlEnabled" class="mt-2 text-[11px] text-muted">
                  Runtime control is disabled; set MCP_TOKEN to enable it.
                </p>
                <p v-else class="mt-2 text-[11px] text-muted">
                  Runtime control needs a reachable MCP endpoint.
                </p>
                <p v-if="logLevelError" class="mt-2 text-[11px] text-error" data-testid="log-level-error">
                  {{ logLevelError }}
                </p>
              </div>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Plugins</h3>
              <div class="space-y-2">
                <article v-for="plugin in detail.plugins" :key="plugin.order" class="rounded-xl border border-border bg-surface-2/50 p-3">
                  <div class="flex items-center gap-2">
                    <span class="mono text-xs">{{ plugin.kind }}</span>
                    <StatusDot :status="plugin.enabled ? 'ok' : 'idle'" :label="plugin.enabled ? 'enabled' : 'disabled'" />
                  </div>
                  <pre class="mono mt-2 whitespace-pre-wrap break-all text-[10px] text-muted">{{ JSON.stringify(plugin.details) }}</pre>
                </article>
              </div>
            </section>

            <section>
              <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Config (read-only)</h3>
              <p v-if="detail.redactedKeys.length" class="mb-2 text-[11px] text-muted">
                {{ detail.redactedKeys.length }} sensitive value(s) redacted:
                <span class="mono">{{ detail.redactedKeys.join(', ') }}</span>
              </p>
              <ConfigPanel v-if="detail.configToml" :toml="detail.configToml" />
              <p v-else class="text-xs text-muted">MCP did not return config.</p>
            </section>
          </template>
        </div>
      </aside>
    </div>
  </div>
</template>
