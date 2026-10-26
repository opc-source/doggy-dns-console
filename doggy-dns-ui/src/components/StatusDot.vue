<script setup lang="ts">
type Status = 'ok' | 'warn' | 'error' | 'idle'

const props = withDefaults(
  defineProps<{
    status?: Status
    label?: string
  }>(),
  { status: 'idle', label: '' },
)

const color: Record<Status, string> = {
  ok: 'bg-ok',
  warn: 'bg-warn',
  error: 'bg-error',
  idle: 'bg-muted',
}
</script>

<template>
  <span class="inline-flex items-center gap-2 text-xs text-muted">
    <span class="h-1.5 w-1.5 rounded-full" :class="color[props.status]" aria-hidden="true" />
    <span v-if="props.label">{{ props.label }}</span>
  </span>
</template>
