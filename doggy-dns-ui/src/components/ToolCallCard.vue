<script setup lang="ts">
import { computed } from 'vue'

import type { ToolCall } from '@/composables/useAgUiStream'

const props = defineProps<{ call: ToolCall }>()

const accent = computed(() => (props.call.name.startsWith('oo_') ? 'text-info' : 'text-accent'))
const summary = computed(() => {
  const raw = props.call.args || props.call.result || ''
  return raw.length > 220 ? `${raw.slice(0, 217)}…` : raw || 'no arguments'
})
const label = computed(() => (props.call.name.startsWith('oo_') ? 'raw telemetry' : 'dns semantics'))
</script>

<template>
  <details class="rounded-xl border border-border bg-surface-2/60 px-3 py-2 text-xs">
    <summary class="flex cursor-pointer list-none items-center gap-2">
      <span class="h-1.5 w-1.5 rounded-full" :class="call.status === 'done' ? 'bg-ok' : call.status === 'error' ? 'bg-error' : 'animate-pulse bg-warn'" />
      <span class="mono font-medium" :class="accent">{{ call.name }}</span>
      <span class="rounded-full border border-border px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-muted">
        {{ label }}
      </span>
      <span class="ml-auto text-[10px] text-muted">{{ call.status }}</span>
    </summary>
    <pre class="mono mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all text-[10px] text-muted">{{ summary }}</pre>
  </details>
</template>
