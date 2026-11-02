<script setup lang="ts">
import { computed } from 'vue'

import type { ToolSegment } from '@/lib/chatModel'

const props = defineProps<{ call: ToolSegment }>()

const accent = computed(() =>
  props.call.name.startsWith('openobserve_') ? 'text-info' : 'text-accent',
)
const label = computed(() =>
  props.call.name.startsWith('openobserve_') ? 'raw telemetry' : 'dns semantics',
)

const args = computed(() => pretty(props.call.args))
const result = computed(() => (props.call.result ? pretty(props.call.result) : ''))

function pretty(raw: string): string {
  const trimmed = raw.trim()
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    try {
      const parsed: unknown = JSON.parse(trimmed)
      if (parsed !== null && typeof parsed === 'object') {
        return cap(JSON.stringify(parsed, null, 2))
      }
    } catch {
      // A truncated stream emits partial JSON; show it as it arrived.
    }
  }
  return cap(trimmed)
}

function cap(value: string): string {
  return value.length > 4000 ? `${value.slice(0, 4000)}…` : value
}
</script>

<template>
  <details class="rounded-xl border border-border bg-surface-2/60 px-3 py-2 text-xs">
    <summary class="flex cursor-pointer list-none items-center gap-2">
      <span
        class="h-1.5 w-1.5 rounded-full"
        :class="call.status === 'done' ? 'bg-ok' : call.status === 'error' ? 'bg-error' : 'animate-pulse bg-warn'"
      />
      <span class="mono font-medium" :class="accent">{{ call.name }}</span>
      <span class="rounded-full border border-border px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-muted">
        {{ label }}
      </span>
      <span class="ml-auto text-[10px] text-muted">{{ call.status }}</span>
    </summary>
    <div class="mt-2 space-y-2">
      <pre
        v-if="args"
        class="mono max-h-40 overflow-auto whitespace-pre-wrap break-all text-[10px] text-muted"
        data-test="tool-args"
      >{{ args }}</pre>
      <pre
        v-if="result"
        class="mono max-h-40 overflow-auto whitespace-pre-wrap break-all text-[10px] text-text/80"
        data-test="tool-result"
      >{{ result }}</pre>
    </div>
  </details>
</template>
