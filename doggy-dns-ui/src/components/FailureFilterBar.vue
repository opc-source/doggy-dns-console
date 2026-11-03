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
  invalid: string | null
  instances: string[]
}>()

const emit = defineEmits<{ 'update:modelValue': [draft: FilterDraft] }>()

const RESPONSE_CODES = ['SERVFAIL', 'REFUSED', 'FORMERR', 'NOERROR']
const QUICK_WINDOWS = [
  { label: '1 小时', minutes: 60, testId: 'quick-1h' },
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

function setFrom(value: string) {
  patch({ fromMs: datetimeToMs(value), toMs: props.modelValue.toMs, windowMinutes: null })
}

function setTo(value: string) {
  patch({ toMs: datetimeToMs(value), windowMinutes: null })
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
  <section class="panel flex flex-wrap items-end gap-3 px-4 py-3">
    <div class="flex gap-2">
      <button
        v-for="preset in QUICK_WINDOWS"
        :key="preset.minutes"
        :data-testid="preset.testId"
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
        :value="fromText"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="setFrom(($event.target as HTMLInputElement).value)"
      />
    </label>
    <label class="flex flex-col gap-1 text-[11px] text-muted">
      止
      <input
        type="datetime-local"
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

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      实例
      <select
        data-testid="instance-select"
        :value="modelValue.instance"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ instance: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部主机</option>
        <option v-for="host in instances" :key="host" :value="host">{{ host }}</option>
      </select>
    </label>

    <label class="flex flex-col gap-1 text-[11px] text-muted">
      响应码
      <select
        :value="modelValue.responseCode"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @change="patch({ responseCode: ($event.target as HTMLSelectElement).value })"
      >
        <option value="">全部</option>
        <option v-for="code in RESPONSE_CODES" :key="code" :value="code">{{ code }}</option>
      </select>
    </label>

    <label class="flex min-w-40 flex-1 flex-col gap-1 text-[11px] text-muted">
      域名包含
      <input
        :value="modelValue.domain"
        placeholder="nacos.local"
        class="focus-ring rounded-lg bg-surface-2 px-2 py-1 text-xs text-text"
        @input="patch({ domain: ($event.target as HTMLInputElement).value })"
      />
    </label>

    <!--
      The banner belongs to the page, so the bar's own line repeats it inline for
      the control that caused it without claiming the page's test id.
    -->
    <p v-if="invalid" class="w-full text-xs text-error">{{ invalid }}</p>
    <p v-else data-testid="range-hint" class="w-full text-[11px] text-muted">
      <!--
        Both ceilings are interpolated, never typed: the investigation chain
        rejects outside 5–10080 (`Range::checked` in `src/domain.rs`), while the
        live overview/instance views use `Window::new` (`src/domain.rs:13`), a
        CLAMP at 5–1440 — a different mechanism and a different type, so the
        number below is documentation of that clamp rather than a shared
        constant, and there is no MAX_WINDOW_MINUTES to import.
      -->
      回溯链支持 {{ MIN_RANGE_MINUTES }} 分钟到 {{ MAX_RANGE_MINUTES }} 分钟（7 天）；看板与实例视图仍限
      1440 分钟。
    </p>
  </section>
</template>
