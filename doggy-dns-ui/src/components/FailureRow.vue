<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'

import type { FailureGroup } from '@/api/types'
import { msToDatetime } from '@/lib/failureFilter'

const props = defineProps<{
  group: FailureGroup
  analyzed: boolean
  /**
   * The overview card's one-line row; false is the page's full field set. Required,
   * and `FailureList` always forwards its own default: one knob, one source of truth.
   */
  dense: boolean
}>()

const emit = defineEmits<{ select: [group: FailureGroup] }>()

// The dense card row has one line of room, so the name is cut at 42 characters to a
// 39-character prefix; the page row shows it whole and wraps instead.
const DENSE_QUERY_MAX_LENGTH = 42
const DENSE_QUERY_PREFIX_LENGTH = 39

function shortName(query: string) {
  if (!props.dense || query.length <= DENSE_QUERY_MAX_LENGTH) return query
  return `${query.slice(0, DENSE_QUERY_PREFIX_LENGTH)}…`
}

/** Every grouped field is optional on the wire, and `null` is not a label. */
function dash(value: string | number | null): string {
  return value === null || value === '' ? '—' : String(value)
}
</script>

<template>
  <button
    type="button"
    data-testid="failure-row"
    class="focus-ring flex w-full items-center gap-3 border-b border-border/60 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-surface-2"
    @click="emit('select', group)"
  >
    <span
      class="mono rounded-md px-1.5 py-0.5 text-[10px]"
      :class="group.responseCode === 'SERVFAIL' ? 'bg-error/15 text-error' : 'bg-warn/15 text-warn'"
    >
      {{ group.responseCode }}
    </span>
    <span
      class="mono min-w-0 flex-1 text-xs text-text"
      :class="dense ? 'truncate' : 'break-all'"
      >{{ shortName(group.query) }}</span
    >

    <template v-if="!dense">
      <span data-testid="row-instance" class="mono w-32 shrink-0 text-xs text-text">{{
        dash(group.instance)
      }}</span>
      <span class="mono w-40 shrink-0 truncate text-[10px] text-muted" :title="group.authority ?? ''">{{
        dash(group.authority)
      }}</span>
      <span class="w-40 shrink-0 truncate text-[10px] text-muted" :title="group.error ?? ''">{{
        dash(group.error)
      }}</span>
      <span class="tabular-nums w-72 shrink-0 text-[10px] text-muted"
        >{{ msToDatetime(group.firstSeen) }} ~ {{ msToDatetime(group.lastSeen) }}</span
      >
      <span class="tabular-nums w-16 shrink-0 text-right text-[10px] text-muted">{{
        group.durationMs === null ? '—' : `${group.durationMs} ms`
      }}</span>
    </template>

    <span class="tabular-nums text-xs text-muted">{{ group.count }}</span>
    <span
      v-if="analyzed"
      data-testid="analyzed-badge"
      class="rounded-md bg-accent/15 px-1.5 py-0.5 text-[10px] text-accent"
      title="已有调查分析"
      >已有分析</span
    >
    <ChevronRight :size="14" class="text-muted" />
  </button>
</template>
