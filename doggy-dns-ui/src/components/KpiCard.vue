<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    label: string
    value: string | number | null
    hint?: string
    format?: 'raw' | 'number' | 'percent' | 'ms' | 'ratio'
    tone?: 'default' | 'ok' | 'warn' | 'error'
  }>(),
  { hint: '', format: 'raw', tone: 'default' },
)

const display = computed(() => {
  if (props.value === null) return '—'
  if (typeof props.value === 'string') return props.value
  switch (props.format) {
    case 'number':
      return new Intl.NumberFormat('en-US').format(props.value)
    case 'percent':
      return `${(props.value * 100).toFixed(1)}%`
    case 'ms':
      return `${props.value.toFixed(1)} ms`
    case 'ratio':
      return String(props.value)
    default:
      return String(props.value)
  }
})

const toneClass = computed(() => {
  switch (props.tone) {
    case 'ok':
      return 'text-ok'
    case 'warn':
      return 'text-warn'
    case 'error':
      return 'text-error'
    default:
      return 'text-text'
  }
})
</script>

<template>
  <article class="panel p-4">
    <p class="text-[11px] font-medium uppercase tracking-[0.14em] text-muted">{{ label }}</p>
    <p class="mt-2 text-2xl font-semibold tracking-tight tabular-nums" :class="toneClass">
      {{ display }}
    </p>
    <p v-if="hint" class="mt-1 text-xs text-muted">{{ hint }}</p>
  </article>
</template>
