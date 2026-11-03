<script setup lang="ts">
import { computed } from 'vue'

import {
  DEFAULT_WINDOW_MINUTES,
  MAX_RANGE_MINUTES,
  MIN_RANGE_MINUTES,
  datetimeToMs,
  msToDatetime,
  relativeDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

const props = defineProps<{
  modelValue: FilterDraft
}>()

const emit = defineEmits<{ 'update:modelValue': [draft: FilterDraft] }>()

// One preset table for all three pages: the dashboard's 6h chip folded in here.
const QUICK_WINDOWS = [
  { label: '1 小时', minutes: 60, testId: 'quick-1h' },
  { label: '6 小时', minutes: 360, testId: 'quick-6h' },
  { label: '24 小时', minutes: 1440, testId: 'quick-24h' },
  { label: '7 天', minutes: MAX_RANGE_MINUTES, testId: 'quick-7d' },
]

function patch(next: Partial<FilterDraft>) {
  emit('update:modelValue', { ...props.modelValue, ...next })
}

const fromText = computed(() =>
  props.modelValue.fromMs === null ? '' : msToDatetime(props.modelValue.fromMs),
)
const toText = computed(() =>
  props.modelValue.toMs === null ? '' : msToDatetime(props.modelValue.toMs),
)

// A bracket must never leave the control half-entered: `toRouteQuery` and `toRequestQuery`
// emit `from`/`to` only as a pair, so a one-sided draft drops the typed date from the
// URL and queries the server default window behind the operator's back. Completing the
// pair here is what keeps the `failureFilter.ts` contract untouched.
function setFrom(value: string) {
  const fromMs = datetimeToMs(value)
  // A cleared input is not half a bracket: both sides go, and the relative
  // window falls back to the server's default so the URL is never empty.
  if (fromMs === null) {
    patch({ fromMs: null, toMs: null, windowMinutes: DEFAULT_WINDOW_MINUTES })
    return
  }
  patch({ fromMs, toMs: props.modelValue.toMs ?? Date.now(), windowMinutes: null })
}

function setTo(value: string) {
  const toMs = datetimeToMs(value)
  if (toMs === null) {
    patch({ fromMs: null, toMs: null, windowMinutes: DEFAULT_WINDOW_MINUTES })
    return
  }
  patch({
    toMs,
    fromMs: props.modelValue.fromMs ?? toMs - DEFAULT_WINDOW_MINUTES * 60_000,
    windowMinutes: null,
  })
}

// A quick window is a fresh relative draft: it must not keep the absolute pair,
// or the resolved bracket would stay the old fixed one.
function useRelative(minutes: number) {
  emit('update:modelValue', {
    ...relativeDraft(minutes),
    domain: props.modelValue.domain,
    responseCode: props.modelValue.responseCode,
    instance: props.modelValue.instance,
  })
}

function resetTime() {
  emit('update:modelValue', {
    ...props.modelValue,
    fromMs: null,
    toMs: null,
    windowMinutes: DEFAULT_WINDOW_MINUTES,
  })
}

const activeQuick = computed(() =>
  !props.modelValue.fromMs && !props.modelValue.toMs ? props.modelValue.windowMinutes : null,
)
</script>

<template>
  <div class="flex gap-2">
    <button
      v-for="preset in QUICK_WINDOWS"
      :key="preset.minutes"
      :data-testid="preset.testId"
      :aria-pressed="activeQuick === preset.minutes"
      class="focus-ring rounded-lg px-2.5 py-1 text-xs"
      :class="
        activeQuick === preset.minutes
          ? 'bg-accent/15 text-accent'
          : 'bg-surface-2 text-muted hover:text-text'
      "
      @click="useRelative(preset.minutes)"
    >
      {{ preset.label }}
    </button>
  </div>

  <label class="flex flex-col gap-1 text-[11px] text-muted">
    起
    <input
      type="datetime-local"
      data-testid="range-from"
      :value="fromText"
      class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
      @change="setFrom(($event.target as HTMLInputElement).value)"
    />
  </label>
  <label class="flex flex-col gap-1 text-[11px] text-muted">
    止
    <input
      type="datetime-local"
      data-testid="range-to"
      :value="toText"
      class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
      @change="setTo(($event.target as HTMLInputElement).value)"
    />
  </label>
  <button
    data-testid="reset-time"
    class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-muted hover:text-text"
    @click="resetTime"
  >
    回到近 1 小时
  </button>

  <p data-testid="range-hint" class="w-full text-[11px] text-muted">
    <!--
      One class now: every endpoint that takes a time range rejects outside
      5–10080 minutes instead of clamping (`Range::resolve` in `src/domain.rs`,
      reached from `RangeQuery::range`, `FailureQuery` and each copilot tool), so
      the numbers below are the request contract, not a per-page ceiling. The one
      remaining `Window::new` clamp lives in internal host discovery
      (`src/datasource/instances.rs`), which no request parameter reaches.
      `/api/instances` is the one endpoint with no window at all — its rows are
      identity and reachability, so the hint has to name it or an operator will
      read a frozen instance list as a stale one.
      注释里写死数字、正文插值:模板注释不参与编译,别在这里放 mustache。
    -->
    时间区间支持 {{ MIN_RANGE_MINUTES }} 分钟到 {{ MAX_RANGE_MINUTES }} 分钟（7 天），所有端点超限直接拒绝、不再夹回；
    实例列表（/api/instances）不取时间窗。
  </p>
</template>
