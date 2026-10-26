<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, toRef } from 'vue'
import { Copy, Sparkles, X } from '@lucide/vue'

import { useFailureDetail } from '@/composables/useFailures'
import { useChatStore } from '@/stores/chat'

const props = defineProps<{ traceId: string }>()
const emit = defineEmits<{ close: [] }>()
const chat = useChatStore()
const trace = toRef(props, 'traceId')
const { detail, error, loading } = useFailureDetail(trace)
const copied = ref(false)
const closeButton = ref<HTMLButtonElement | null>(null)
const dialog = ref<HTMLElement | null>(null)
let previousFocus: Element | null = null

const title = computed(() => detail.value?.group.query ?? props.traceId)
const prompt = computed(() => {
  const group = detail.value?.group
  if (!group) return `请分析这组失败：trace_id=${props.traceId}`
  return [
    '请分析这组 DNS 失败：',
    `trace_id=${props.traceId}`,
    `域名=${group.query}`,
    `类型=${group.qtype}`,
    `错误码=${group.responseCode}`,
    group.authority ? `authority=${group.authority}` : '',
    group.instance ? `实例=${group.instance}` : '',
    group.error ? `错误=${group.error}` : '',
  ]
    .filter(Boolean)
    .join('，')
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

onMounted(() => {
  previousFocus = document.activeElement
  window.addEventListener('keydown', onKeydown)
  void nextTick(() => closeButton.value?.focus())
})
onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
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

async function analyze() {
  const sent = await chat.send(prompt.value)
  if (sent) emit('close')
}

function when(ts: number) {
  return new Date(ts).toLocaleString()
}
</script>

<template>
  <div class="fixed inset-0 z-40 flex justify-end bg-black/40 backdrop-blur-[2px]" @click.self="emit('close')">
    <aside class="flex h-full w-full max-w-[480px] flex-col border-l border-border bg-surface shadow-2xl" data-testid="failure-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="Failure detail">
      <header class="border-b border-border px-4 py-4">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <p class="mono text-[10px] uppercase tracking-[0.16em] text-error">failure trace</p>
            <h2 class="mono mt-1 truncate text-sm text-text">{{ title }}</h2>
          </div>
          <button ref="closeButton" class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border text-muted hover:text-text" aria-label="Close" @click="emit('close')">
            <X :size="15" />
          </button>
        </div>
        <div class="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-muted">
          <button class="focus-ring mono flex items-center gap-1 rounded-lg border border-border px-2 py-1 hover:text-text" @click="copyTrace">
            <Copy :size="11" /> {{ copied ? 'copied' : props.traceId }}
          </button>
          <span v-if="detail" class="rounded-md bg-error/15 px-1.5 py-0.5 text-error">{{ detail.group.responseCode }}</span>
          <span v-if="detail" class="tabular-nums">×{{ detail.group.count }}</span>
          <span v-if="detail" class="mono">{{ detail.group.instance }}</span>
        </div>
      </header>

      <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <p v-if="loading" class="text-xs text-muted">Loading trace…</p>
        <p v-else-if="error" class="rounded-lg border border-error/30 bg-error/10 px-3 py-2 text-xs text-error">{{ error }}</p>
        <template v-else-if="detail">
          <section>
            <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Logs</h3>
            <div class="space-y-2">
              <article v-for="(log, index) in detail.logs" :key="index" class="rounded-xl border border-border bg-surface-2/50 p-3">
                <div class="flex items-center gap-2 text-[10px] text-muted">
                  <span class="rounded bg-error/15 px-1.5 py-0.5 text-error">{{ log.severity }}</span>
                  <span class="mono">{{ when(log.ts) }}</span>
                </div>
                <p class="mt-2 text-xs text-text">{{ log.message }}</p>
                <pre class="mono mt-2 whitespace-pre-wrap break-all text-[10px] text-muted">{{ JSON.stringify(log.fields, null, 2) }}</pre>
              </article>
            </div>
          </section>

          <section>
            <h3 class="mb-2 text-[11px] uppercase tracking-[0.14em] text-muted">Spans</h3>
            <div class="space-y-2">
              <article v-for="span in detail.spans" :key="span.spanId" class="rounded-xl border border-border bg-surface-2/50 p-3">
                <div class="flex items-center gap-2">
                  <span class="h-1.5 w-1.5 rounded-full bg-error" />
                  <span class="mono text-xs text-text">{{ span.name }}</span>
                  <span class="ml-auto tabular-nums text-[10px] text-muted">{{ span.durationMs?.toFixed(1) ?? '—' }} ms</span>
                </div>
                <p class="mono mt-1 text-[10px] text-muted">{{ span.spanId }} · {{ span.status }}</p>
              </article>
            </div>
          </section>
        </template>
      </div>

      <footer class="border-t border-border p-4">
        <button
          data-test="analyze"
          class="focus-ring flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-2.5 text-xs font-semibold text-bg hover:opacity-90 disabled:opacity-40"
          :disabled="!detail"
          @click="analyze"
        >
          <Sparkles :size="14" /> 让 AI 分析这组失败
        </button>
      </footer>
    </aside>
  </div>
</template>
