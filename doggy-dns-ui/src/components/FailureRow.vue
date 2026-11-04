<script setup lang="ts">
import { computed } from 'vue'
import { ChevronRight } from '@lucide/vue'

import type { AnalysisMarker, FailureGroup } from '@/api/types'
import { msToDatetime } from '@/lib/failureFilter'

const props = defineProps<{
  group: FailureGroup
  /** The latest attempt for this trace, when one exists. */
  marker?: AnalysisMarker
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

/** Every grouped field is optional on the wire, and neither null nor an omission is a label. */
function dash(value: string | number | null | undefined): string {
  return value === null || value === undefined || value === '' ? '—' : String(value)
}

// The badge names the attempt the row links to: an in-flight run is not yet an
// answer, and a failed or illegal one must not read like a finished conclusion.
const badgeText = computed(() => {
  const marker = props.marker
  if (!marker) return ''
  if (marker.status === 'queued' || marker.status === 'running') return '分析中'
  if (marker.status === 'failed') return '已有分析 · 失败'
  if (marker.status === 'invalid') return '已有分析 · 结果非法'
  return marker.verdictCategory ? `已有分析 · ${marker.verdictCategory}` : '已有分析'
})

const badgeTitle = computed(() => {
  const marker = props.marker
  if (!marker) return ''
  return `分析 ${marker.id} · 第 ${marker.attempt} 次 · ${msToDatetime(marker.createdAt)}`
})
</script>

<template>
  <!--
    The page row splits into two lines on purpose. Nine `shrink-0` spans plus the
    query need ~1014px, and the row's content box is ~874px at a 1440px viewport,
    so on one line the query — the only shrinkable child — collapses to 0 width and
    `break-all` paints it one character per line. `contents` keeps the dense card
    row's single line byte-identical: its children stay direct flex items.
  -->
  <button
    type="button"
    data-testid="failure-row"
    class="focus-ring w-full border-b border-border/60 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-surface-2"
    :class="dense ? 'flex items-center gap-3' : 'flex flex-col gap-1.5'"
    @click="emit('select', group)"
  >
    <span :class="dense ? 'contents' : 'flex min-w-0 items-center gap-3'">
      <span
        class="mono rounded-md px-1.5 py-0.5 text-[10px]"
        :class="group.responseCode === 'SERVFAIL' ? 'bg-error/15 text-error' : 'bg-warn/15 text-warn'"
      >
        {{ group.responseCode }}
      </span>
      <span class="mono rounded-md bg-surface-2 px-1.5 py-0.5 text-[10px] text-muted">{{
        group.qtype
      }}</span>
      <span
        data-testid="row-query"
        class="mono min-w-0 flex-1 text-xs text-text"
        :class="dense ? 'truncate' : 'break-all'"
        >{{ shortName(group.query) }}</span
      >
      <span class="tabular-nums text-xs text-muted">{{ group.count }}</span>
      <span
        v-if="marker"
        data-testid="analyzed-badge"
        class="rounded-md bg-accent/15 px-1.5 py-0.5 text-[10px] text-accent"
        :title="badgeTitle"
        >{{ badgeText }}</span
      >
      <ChevronRight :size="14" class="text-muted" />
    </span>

    <span v-if="!dense" data-testid="row-meta" class="flex flex-wrap items-center gap-3">
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
    </span>
  </button>
</template>
