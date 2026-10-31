<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'

import type { FailureGroup } from '@/api/types'

defineProps<{ groups: FailureGroup[]; markers?: Set<string> }>()
defineEmits<{ select: [group: FailureGroup] }>()

function shortName(query: string) {
  return query.length > 42 ? `${query.slice(0, 39)}…` : query
}
</script>

<template>
  <div class="panel overflow-hidden">
    <header class="flex items-center justify-between border-b border-border px-4 py-3">
      <h2 class="text-sm font-medium">Top failures</h2>
      <span class="text-xs text-muted">last window</span>
    </header>
    <div v-if="groups.length === 0" class="px-4 py-10 text-center text-sm text-muted">
      No failures in this window.
    </div>
    <button
      v-for="group in groups.slice(0, 5)"
      :key="group.key"
      data-testid="failure-row"
      class="focus-ring flex w-full items-center gap-3 border-b border-border/60 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-surface-2"
      @click="$emit('select', group)"
    >
      <span
        class="mono rounded-md px-1.5 py-0.5 text-[10px]"
        :class="group.responseCode === 'SERVFAIL' ? 'bg-error/15 text-error' : 'bg-warn/15 text-warn'"
      >
        {{ group.responseCode }}
      </span>
      <span class="mono min-w-0 flex-1 truncate text-xs text-text">{{ shortName(group.query) }}</span>
      <span class="tabular-nums text-xs text-muted">{{ group.count }}</span>
      <span
        v-if="markers?.has(group.traceId ?? '')"
        data-testid="analyzed-badge"
        class="rounded-md bg-accent/15 px-1.5 py-0.5 text-[10px] text-accent"
        title="已有调查分析"
        >已有分析</span
      >
      <ChevronRight :size="14" class="text-muted" />
    </button>
  </div>
</template>
