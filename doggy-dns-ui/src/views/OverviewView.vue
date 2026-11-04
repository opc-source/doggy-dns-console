<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import ErrorBanner from '@/components/ErrorBanner.vue'
import FailureList from '@/components/FailureList.vue'
import KpiCard from '@/components/KpiCard.vue'
import TimeRangeControl from '@/components/TimeRangeControl.vue'
import TrendChart from '@/components/TrendChart.vue'
import { useFailureList } from '@/composables/useFailureList'
import { useOverview } from '@/composables/useOverview'
import {
  DEFAULT_WINDOW_MINUTES,
  parseFilterQuery,
  rangeLabel,
  relativeDraft,
  toRouteQuery,
  type FilterDraft,
} from '@/lib/failureFilter'

const route = useRoute()
const router = useRouter()

// `route.query` is a `LocationQuery`,数组成员可能为 null;`parseFilterQuery` 只取
// 第一个字符串,所以两边运行时对得上,只需要一处不加宽度的转换。
type FilterQuery = Parameters<typeof parseFilterQuery>[0]

// URL 是唯一状态来源(§6.5):draft 是派生的,写入只回到路由,所以这里不需要
// `FailuresView.vue` 那套"序列化后双向比对"的防乒乓守卫——那套是给本地可编辑的
// `ref<FilterDraft>` 用的,派生 computed 没有可以偏离 URL 的本地态。
const draft = computed(() => {
  const parsed = parseFilterQuery(route.query as FilterQuery)
  // 一条时间参数都没有时不是"无窗口",而是"服务端默认那一小时";在这里补上,
  // `TimeRangeControl` 的 `aria-pressed` 才有归属,而 URL 保持干净。
  // 不去 `onMounted` 里 `router.replace` 补 URL:那会让首屏发两个请求
  // (一次无参、一次带 windowMinutes),深链测试里"只发一次"就永远测不出来。
  if (parsed.fromMs === null && parsed.toMs === null && parsed.windowMinutes === null) {
    return relativeDraft(DEFAULT_WINDOW_MINUTES)
  }
  return parsed
})

function setDraft(next: FilterDraft) {
  void router.replace({ query: toRouteQuery(next) })
}

const { data, error, invalid, loading, lastUpdated, refresh } = useOverview(draft)
const { groups: failures, markers } = useFailureList(draft, 5)

function onSelect(traceId: string) {
  // The card's own bracket travels with the click, so the drawer investigates the
  // range the operator was looking at instead of a hard-coded one.
  void router.push({
    name: 'failure-detail',
    params: { traceId },
    query: toRouteQuery(draft.value),
  })
}

const successRate = computed(() => data.value?.successRate ?? null)
const errorMessage = computed(() => {
  // A refusal has no previous success to keep behind it, so it must not read as
  // "上次成功数据"; that copy is reserved for a server error over live data.
  if (invalid.value) return invalid.value
  if (!error.value) return ''
  if (data.value) return 'OpenObserve 暂不可用，展示的是上次成功数据'
  return '无法加载 Overview，请检查 OpenObserve 连接'
})
</script>

<template>
  <div class="mx-auto flex max-w-[1440px] flex-col gap-4 p-5 lg:p-7">
    <header class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="mono text-[11px] uppercase tracking-[0.18em] text-accent">doggy dns</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight lg:text-3xl">Overview</h1>
        <p data-testid="range-label" class="mt-1 text-sm text-muted">
          {{ rangeLabel(draft, Date.now()) }}
        </p>
      </div>
      <div class="flex items-center gap-2">
        <TimeRangeControl :model-value="draft" @update:model-value="setDraft" />
        <button
          class="focus-ring grid h-8 w-8 place-items-center rounded-xl border border-border bg-surface text-muted hover:text-text disabled:opacity-50"
          :disabled="loading"
          aria-label="Refresh"
          @click="refresh"
        >
          <span :class="{ 'animate-spin': loading }">⟳</span>
        </button>
      </div>
    </header>

    <ErrorBanner
      v-if="error || invalid"
      :message="errorMessage"
      :stale-at="data ? lastUpdated : null"
      @retry="refresh"
    />

    <section class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <KpiCard
        label="Instances online"
        :value="data ? `${data.instancesOnline} / ${data.instancesTotal}` : null"
        hint="MCP + telemetry"
      />
      <KpiCard
        label="Queries"
        :value="data?.queries ?? null"
        format="number"
        :hint="rangeLabel(draft, Date.now())"
      />
      <KpiCard
        label="Success rate"
        :value="successRate"
        format="percent"
        :tone="successRate !== null && successRate < 0.99 ? 'warn' : 'ok'"
        hint="NXDOMAIN excluded"
      />
      <KpiCard
        label="P95 latency"
        :value="data?.p95LatencyMs ?? null"
        format="ms"
        hint="worst bucket in range"
      />
    </section>

    <section class="grid grid-cols-1 gap-3 xl:grid-cols-[1.5fr_1fr]">
      <TrendChart v-if="data" :series="data.series" />
      <div v-else class="panel h-[300px] animate-pulse bg-surface-2/40" />
      <FailureList
        :groups="failures"
        :markers="markers"
        :limit="5"
        see-all
        :see-all-to="{ name: 'failures-list', query: toRouteQuery(draft) }"
        @select="(group) => group.traceId && onSelect(group.traceId)"
      />
    </section>

    <section class="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <div class="panel p-4">
        <h2 class="text-sm font-medium">What changed?</h2>
        <p class="mt-2 text-sm text-muted">
          AI analysis is docked on the right. Ask about the current window, or open a failure to
          hand its trace to the copilot.
        </p>
        <p class="mt-4 rounded-xl border border-border bg-surface-2 px-3 py-2 text-xs text-muted">
          Tip: open a failure to load its evidence — analysis starts automatically, and the
          share link restores it later.
        </p>
      </div>
      <div class="panel p-4">
        <h2 class="text-sm font-medium">Failure semantics</h2>
        <ul class="mt-2 space-y-1 text-xs text-muted">
          <li>SERVFAIL / REFUSED / FORMERR / timeout count as failures.</li>
          <li>NXDOMAIN is a miss, not a failure.</li>
          <li>Success rate is undefined when there is no traffic.</li>
        </ul>
      </div>
    </section>
  </div>
</template>
